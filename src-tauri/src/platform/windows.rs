//! Windows 平台实现：全部通过 PowerShell + 内嵌 C# (Win32 P/Invoke) 落地，
//! 延续项目「零额外 Rust 依赖」的风格。每个命令独立 powershell 进程，
//! C# 代码用单引号包裹传入 Add-Type（C# 内不含单引号字符）。

use super::{run_cmd, DisplayInfo};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub fn platform_name() -> &'static str {
    "windows"
}

/// 是否已有更早启动的同名实例在跑（防双开 / 防开出多个托盘图标）。
/// 用 PID 比较而不是单纯计数：两个实例几乎同时启动时会「各自都看到 2 个进程」，
/// 若只判断数量会双双退出；改为「PID 更小的那个留下」，保证恰好一个存活。
pub fn another_instance_running() -> bool {
    let me = std::process::id();
    let script = format!(
        "$p = Get-Process -Name screenguard -ErrorAction SilentlyContinue | Where-Object {{ $_.Id -lt {} }}; if ($p) {{ '1' }} else {{ '0' }}",
        me
    );
    run_cmd(PS, &["-NoProfile", "-Command", script.as_str()])
        .map(|o| o.trim() == "1")
        .unwrap_or(false)
}

const PS: &str = "powershell";

// ===== 开机自启（HKCU Run 注册表项）=====
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "Screenguard";

fn current_exe_path() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("无法定位程序自身路径：{}", e))
}

pub fn autostart_enabled() -> bool {
    std::process::Command::new("reg")
        .args(["query", RUN_KEY, "/v", RUN_NAME])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let exe = current_exe_path()?;
    if enabled {
        // 路径必须带引号：Run 键的值含空格时（如 C:\Program Files\...）无引号会被
        // Windows 按空格切分，可能误去启动 C:\Program.exe 导致自启静默失效
        let quoted = format!("\"{}\"", exe);
        run_cmd(
            "reg",
            &["add", RUN_KEY, "/v", RUN_NAME, "/t", "REG_SZ", "/d", quoted.as_str(), "/f"],
        )
        .map(|_| ())
        .map_err(|e| format!("写入开机自启失败：{}", e.trim()))
    } else {
        // 删除注册表项（不存在时 reg delete 会报错，忽略之）
        let _ = run_cmd("reg", &["delete", RUN_KEY, "/v", RUN_NAME, "/f"]);
        Ok(())
    }
}

fn ps(script: &str) -> Result<String, String> {
    run_cmd(PS, &["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script])
}

/// 统一的 Win32 桥接 C#（显示器枚举 / DEVMODE / DDC/CI / 窗口 / 旋转）
/// 注意：此常量内禁止出现单引号字符，否则会截断 Add-Type 的引号包裹。
const CORE_CS: &str = r#"using System;
using System.Text;
using System.Runtime.InteropServices;

// DEVMODEW 不再用结构体映射，改为按固定偏移直接读写（Marshal.Read/WriteInt32）。
// 原因：① Explicit 布局只声明需要的字段时，StructureToPtr 会把未声明区间（dmDeviceName /
// dmFormName / dmICMIntent 等）清零，读改写回不干净；② Sequential 布局的字段对齐依赖
// marshaller 实现，无法在本机验证。按偏移直读直写既确定又无损。
// 偏移见 SGCore 内的 OFF_* 常量（本机 Win11 24H2 用 ctypes 实测校准，DEVMODEW 共 220 字节）。

[StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
public struct SGDISPLAY_DEVICE {
  public int cb;
  [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string DeviceName;
  [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceString;
  // 文档布局（DISPLAY_DEVICEW，共 840 字节）：StateFlags 紧跟 DeviceString，之后才是
  // DeviceID / DeviceKey。旧实现的「_pad 隐藏字段」是误诊——实测（ctypes 与 .NET 双路验证）
  // Win11 24H2 起本机 EnumDisplayDevicesW 对「适配器上挂的监视器」的枚举直接失败
  // （返回 FALSE），与 cb 取 840/844 无关；uid 因此改由调用方做回退处理（见 ListDisplays）。
  public int StateFlags;
  [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceID;
  [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceKey;
}

public struct SGRECT { public int Left, Top, Right, Bottom; }

[StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
public struct SGMONITORINFO {
  public int cb;
  public SGRECT rcMonitor;
  public SGRECT rcWork;
  public int dwFlags;
  [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string szDevice;
}

[StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
public struct SGPHYS_MON { public IntPtr h; [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string name; }

// ---------- DisplayConfig（CCD）路径查询：GDI 设备名 → 显示器友好名 ----------
// Win11 24H2 实测：EnumDisplayDevicesW 的监视器枚举与 WMI 监视器类可能整体失效，
// DisplayConfig 是与驱动栈状态无关的权威来源。结构布局已对照本机 SDK wingdi.h 核实：
//   SOURCE_INFO=20(无 reserved！) TARGET_INFO=48 PATH_INFO=72
//   SOURCE_NAME=84(头20+WCHAR[32]) TARGET_NAME=420(头20+flags4+outputTechnology4
//   +edidManufactureId2+edidProductCodeId2+connectorInstance4+名WCHAR[64]+路径WCHAR[128])
// 路径数组按偏移直读（每条 72 字节）：
//   +0 源adapterId.Lo +4 源adapterId.Hi +8 源id +20 目标adapterId.Lo +24 目标adapterId.Hi
//   +28 目标id +68 flags(1=ACTIVE)
[StructLayout(LayoutKind.Sequential)]
public struct SGLUID { public uint LowPart; public int HighPart; }

[StructLayout(LayoutKind.Sequential)]
public struct SGDC_HEADER { public uint type; public uint size; public SGLUID adapterId; public uint id; }

[StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
public struct SGDC_SRC_NAME { public SGDC_HEADER header; [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string viewGdiDeviceName; }

[StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
public struct SGDC_DST_NAME {
  public SGDC_HEADER header; public uint flags; public uint outputTechnology;
  public ushort edidManufactureId; public ushort edidProductCodeId; public uint connectorInstance;
  [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 64)] public string monitorFriendlyDeviceName;
  [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string monitorDevicePath;
}

public class SGCore {
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool EnumDisplayDevicesW(string dev, uint i, ref SGDISPLAY_DEVICE d, uint f);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool EnumDisplaySettingsW(string dev, int mode, IntPtr dm);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int ChangeDisplaySettingsExW(string dev, IntPtr dm, IntPtr nul, uint flags, IntPtr p);
  [DllImport("user32.dll")] static extern int GetSystemMetrics(int idx);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool GetMonitorInfo(IntPtr h, ref SGMONITORINFO mi);
  [DllImport("user32.dll")] static extern bool EnumDisplayMonitors(IntPtr h, IntPtr rc, EnumMonProc cb, IntPtr lp);
  delegate bool EnumMonProc(IntPtr h, IntPtr hdc, IntPtr rc, IntPtr lp);

  [DllImport("dxva2.dll")] static extern bool GetNumberOfPhysicalMonitorsFromHMONITOR(IntPtr h, out uint n);
  // 关键：arr 含 string 字段(非 blittable)，必须标 [Out] 才会把 API 写入的句柄/描述复制回托管端。
  // 缺 [Out] 时 .NET 只传临时副本且不回传 → 拿到无效句柄 → DDC 读写全部失败
  // （曾因此误判"显示器不支持 DDC/CI"，同机 monitorcontrol 却能正常读）。
  [DllImport("dxva2.dll")] static extern bool GetPhysicalMonitorsFromHMONITOR(IntPtr h, uint n, [Out] SGPHYS_MON[] arr);
  [DllImport("dxva2.dll")] static extern bool DestroyPhysicalMonitors(uint n, SGPHYS_MON[] arr);
  [DllImport("dxva2.dll")] static extern bool SetVCPFeature(IntPtr h, byte code, uint val);
  // 注意：真实签名是 (h, code, pvct, pdwCurrentValue, pdwMaximumValue)——两个独立 DWORD 输出，
  // 不是结构体。写成结构体会导致输出参数错位、DDC 读写必然全部失败。
  [DllImport("dxva2.dll")] static extern bool GetVCPFeatureAndVCPFeatureReply(IntPtr h, byte code, IntPtr pvct, ref uint cur, ref uint max);

  [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out SGRECT r);
  [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] static extern IntPtr GetWindowLongPtr(IntPtr h, int idx);
  [DllImport("user32.dll", EntryPoint = "SetWindowLongPtrW")] static extern IntPtr SetWindowLongPtr(IntPtr h, int idx, IntPtr v);
  [DllImport("user32.dll")] static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool SystemParametersInfo(uint action, uint p0, ref SGRECT p1, uint f);

  // 色彩配置（ICC）：GDI 设备上下文路径
  [DllImport("gdi32.dll", CharSet = CharSet.Unicode)] static extern IntPtr CreateDCW(string drv, string dev, string port, IntPtr pdm);
  [DllImport("gdi32.dll")] static extern bool DeleteDC(IntPtr hdc);
  [DllImport("gdi32.dll", CharSet = CharSet.Unicode)] static extern bool GetICMProfileW(IntPtr hdc, ref uint size, StringBuilder name);
  [DllImport("gdi32.dll", CharSet = CharSet.Unicode)] static extern bool SetICMProfileW(IntPtr hdc, string file);
  [DllImport("mscms.dll", CharSet = CharSet.Unicode)] static extern bool InstallColorProfileW(IntPtr h, string prof);

  // DisplayConfig（CCD）：活动路径与设备名查询（与 PnP/WMI 监视器状态无关）。
  // 路径/模式数组必须走原始指针：实测 .NET 结构体数组经 marshaller 进出后数据全零
  // （与 dxva2 GetPhysicalMonitorsFromHMONITOR 必须 [Out] 是同一类坑，这里干脆绕开 marshaller）
  [DllImport("user32.dll")] static extern int GetDisplayConfigBufferSizes(uint flags, out uint nPaths, out uint nModes);
  [DllImport("user32.dll", EntryPoint = "QueryDisplayConfig")] static extern int QueryDisplayConfigRaw(uint flags, ref uint nPaths, IntPtr paths, ref uint nModes, IntPtr modes, IntPtr topo);
  [DllImport("user32.dll")] static extern int DisplayConfigGetDeviceInfo(ref SGDC_SRC_NAME pkt);
  [DllImport("user32.dll")] static extern int DisplayConfigGetDeviceInfo(ref SGDC_DST_NAME pkt);

  const int GWL_STYLE = -16;
  const uint WS_CAPTION = 0x00C00000, WS_THICKFRAME = 0x00040000, WS_POPUP = 0x80000000;
  const uint SWP_NOZORDER = 0x0004, SWP_NOACTIVATE = 0x0010, SWP_FRAMECHANGED = 0x0020, SWP_SHOWWINDOW = 0x0040;
  const uint DM_DISPLAYORIENTATION = 0x00000080;
  const uint DM_PELSWIDTH = 0x00080000, DM_PELSHEIGHT = 0x00100000, DM_DISPLAYFREQUENCY = 0x00400000;
  const uint CDS_UPDATEREGISTRY = 0x0001, CDS_GLOBAL = 0x0008;
  const uint SPI_GETWORKAREA = 0x0030;

  // DEVMODEW（220 字节）关键字段偏移，本机 Win11 24H2 用 ctypes 实测校准
  const int DEVMODE_SIZE = 220;
  const int OFF_DMSIZE = 68;
  const int OFF_DMFIELDS = 72;
  const int OFF_ORIENTATION = 84;
  const int OFF_PELSWIDTH = 172;
  const int OFF_PELSHEIGHT = 176;
  const int OFF_FREQUENCY = 184;

  static IntPtr AllocDevMode() { return Marshal.AllocHGlobal(DEVMODE_SIZE); }

  /// 枚举已连接的显示器（EnumDisplayMonitors 路径，兼容各会话/混合显卡）。
  /// 行: D|<dev>|<uid>|<w>|<h>|<hz>|<main>|<ori>
  public static string ListDisplays() {
    var sb = new StringBuilder();
    EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, (h, hdc, rc, lp) => {
      var mi = new SGMONITORINFO();
      mi.cb = Marshal.SizeOf(typeof(SGMONITORINFO));
      if (GetMonitorInfo(h, ref mi)) {
        string dev = mi.szDevice;
        var b = AllocDevMode();
        try {
          if (EnumDisplaySettingsW(dev, -1, b)) {
            // 按偏移直读，不做结构体映射
            uint dw = (uint)Marshal.ReadInt32(b, OFF_PELSWIDTH);
            uint dh = (uint)Marshal.ReadInt32(b, OFF_PELSHEIGHT);
            uint dhz = (uint)Marshal.ReadInt32(b, OFF_FREQUENCY);
            uint dori = (uint)Marshal.ReadInt32(b, OFF_ORIENTATION);
            string uid = "";
            uint j = 0;
            while (true) {
              var m = new SGDISPLAY_DEVICE();
              m.cb = Marshal.SizeOf(typeof(SGDISPLAY_DEVICE));
              if (!EnumDisplayDevicesW(dev, j, ref m, 0)) break;
              uid = m.DeviceID;
              j++;
            }
            // Win11 24H2 起 EnumDisplayDevicesW 不再枚举适配器上挂的监视器（实测恒返回
            // FALSE，cb=840/844 均如此）→ uid 取不到。回退用 dev 名作会话内唯一 id：
            // get_displays 每次全量重建 id↔dev 映射，dev 名在会话内稳定，功能等价。
            if (string.IsNullOrEmpty(uid)) uid = dev;
            int main = (mi.dwFlags & 1) != 0 ? 1 : 0;
            sb.Append("D|").Append(dev).Append("|").Append(uid).Append("|")
              .Append(dw).Append("|").Append(dh).Append("|")
              .Append(dhz).Append("|").Append(main).Append("|")
              .Append(dori).Append("\n");
          }
        } finally { Marshal.FreeHGlobal(b); }
      }
      return true;
    }, IntPtr.Zero);
    return sb.ToString();
  }

  /// 活动显示路径映射：GDI 设备名(\\.\DISPLAYn) → 显示器友好名。
  /// 行: M|<dev>|<name>。失败/无路径返回空串（调用方回退 WMI → 默认命名）。
  public static string DisplayNameMap() {
    uint nPaths, nModes;
    // 注意：QDC_ONLY_ACTIVE_PATHS=2（QDC_ALL_PATHS 才是 1）——拿全部路径会得到几十条
    // 非活动路径，其源设备名是 WinDisc 占位、目标名查询直接报 87
    if (GetDisplayConfigBufferSizes(2 /*QDC_ONLY_ACTIVE_PATHS*/, out nPaths, out nModes) != 0 || nPaths == 0) return "";
    IntPtr pa = Marshal.AllocHGlobal((int)(nPaths * 72));
    IntPtr ma = Marshal.AllocHGlobal((int)(nModes * 64));
    try {
      uint p = nPaths, m = nModes;
      if (QueryDisplayConfigRaw(2, ref p, pa, ref m, ma, IntPtr.Zero) != 0) return "";
      var sb = new StringBuilder();
      var seen = new System.Collections.Generic.HashSet<string>();
      for (int i = 0; i < p; i++) {
        int off = i * 72;
        var src = new SGDC_SRC_NAME();
        src.header.type = 1; src.header.size = (uint)Marshal.SizeOf(typeof(SGDC_SRC_NAME)); // GET_SOURCE_NAME
        src.header.adapterId.LowPart = (uint)Marshal.ReadInt32(pa, off);
        src.header.adapterId.HighPart = Marshal.ReadInt32(pa, off + 4);
        src.header.id = (uint)Marshal.ReadInt32(pa, off + 8);
        if (DisplayConfigGetDeviceInfo(ref src) != 0) continue;
        var dst = new SGDC_DST_NAME();
        dst.header.type = 2; dst.header.size = (uint)Marshal.SizeOf(typeof(SGDC_DST_NAME)); // GET_TARGET_NAME
        dst.header.adapterId.LowPart = (uint)Marshal.ReadInt32(pa, off + 20);
        dst.header.adapterId.HighPart = Marshal.ReadInt32(pa, off + 24);
        dst.header.id = (uint)Marshal.ReadInt32(pa, off + 28);
        if (DisplayConfigGetDeviceInfo(ref dst) != 0) continue;
        string dev = src.viewGdiDeviceName, nm = dst.monitorFriendlyDeviceName;
        if (string.IsNullOrEmpty(dev) || string.IsNullOrEmpty(nm)) continue;
        if (!seen.Add(dev.ToUpperInvariant())) continue;
        sb.Append("M|").Append(dev).Append("|").Append(nm).Append("\n");
      }
      return sb.ToString();
    } finally { Marshal.FreeHGlobal(pa); Marshal.FreeHGlobal(ma); }
  }

  /// 对指定显示设备执行 DDC/CI（VCP 读或写），返回是否命中；retries 为最大重试次数
  static bool TryVCP(string dev, byte code, bool write, uint val, int retries, out uint cur) {
    cur = 0;
    var curBox = new uint[1];
    var anyBox = new bool[1];
    EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, (h, hdc, rc, lp) => {
      var mi = new SGMONITORINFO();
      mi.cb = Marshal.SizeOf(typeof(SGMONITORINFO));
      if (GetMonitorInfo(h, ref mi) && mi.szDevice.Equals(dev, StringComparison.OrdinalIgnoreCase)) {
        uint n;
        if (GetNumberOfPhysicalMonitorsFromHMONITOR(h, out n) && n > 0) {
          var arr = new SGPHYS_MON[n];
          if (GetPhysicalMonitorsFromHMONITOR(h, n, arr)) {
            foreach (var pm in arr) {
              if (write) {
                // DDC 写入偶发失败，重试
                for (int t = 0; t < retries && !anyBox[0]; t++) {
                  if (SetVCPFeature(pm.h, code, val)) anyBox[0] = true;
                  else System.Threading.Thread.Sleep(120);
                }
              } else {
                // 显示器 DDC 响应慢：单次读取常失败，必须重试
                // 注意：变量不能叫 cur/max——与外层参数 out cur 同名的局部变量会 C# 编译失败
                uint curVal = 0, maxVal = 0;
                bool ok = false;
                for (int t = 0; t < retries && !ok; t++) {
                  ok = GetVCPFeatureAndVCPFeatureReply(pm.h, code, IntPtr.Zero, ref curVal, ref maxVal);
                  if (!ok) System.Threading.Thread.Sleep(120);
                }
                if (ok) { curBox[0] = curVal; anyBox[0] = true; }
              }
            }
            DestroyPhysicalMonitors(n, arr);
          }
        }
      }
      return true;
    }, IntPtr.Zero);
    cur = curBox[0];
    return anyBox[0];
  }

  /// 读 VCP 值，失败返回 ERR
  public static string DDCRead(string dev, byte code) {
    uint cur;
    return TryVCP(dev, code, false, 0, 5, out cur) ? cur.ToString() : "ERR";
  }

  /// 写 VCP 值，返回 OK / ERR
  public static string DDCWrite(string dev, byte code, uint val) {
    uint cur;
    return TryVCP(dev, code, true, val, 3, out cur) ? "OK" : "ERR:该显示器不支持 DDC/CI";
  }

  /// 探测是否支持 DDC（VCP 0x10 亮度可读）。重试少：列表刷新会逐屏调用，需快速返回。
  public static string DDCProbe(string dev) {
    uint cur;
    return TryVCP(dev, 0x10, false, 0, 2, out cur) ? "1" : "0";
  }

  /// 旋转副屏。newOri: 0=横, 1=顺时针90, 3=逆时针90
  /// 按 DEVMODEW 固定偏移「就地改」：只写 dmSize / dmFields / 方向 / 宽高，其余字节原样不动。
  /// （旧实现用 Explicit 结构体 PtrToStructure→StructureToPtr 整块回写，未声明区间会被清零）
  public static string Rotate(string dev, uint newOri) {
    var b = AllocDevMode();
    try {
      if (!EnumDisplaySettingsW(dev, -1, b)) return "ERR:读取当前显示模式失败";
      uint w = (uint)Marshal.ReadInt32(b, OFF_PELSWIDTH);
      uint h = (uint)Marshal.ReadInt32(b, OFF_PELSHEIGHT);
      uint curOri = (uint)Marshal.ReadInt32(b, OFF_ORIENTATION);
      bool curP = (curOri == 1 || curOri == 3);
      bool newP = (newOri == 1 || newOri == 3);
      if (curP != newP) { uint t = w; w = h; h = t; }
      Marshal.WriteInt32(b, OFF_ORIENTATION, (int)newOri);
      Marshal.WriteInt32(b, OFF_PELSWIDTH, (int)w);
      Marshal.WriteInt32(b, OFF_PELSHEIGHT, (int)h);
      Marshal.WriteInt32(b, OFF_DMFIELDS, (int)(DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_DISPLAYORIENTATION));
      Marshal.WriteInt16(b, OFF_DMSIZE, (short)DEVMODE_SIZE);
      int r = ChangeDisplaySettingsExW(dev, b, IntPtr.Zero, CDS_UPDATEREGISTRY | CDS_GLOBAL, IntPtr.Zero);
      return r == 0 ? "OK" : ("ERR:ChangeDisplaySettingsEx 返回 " + r);
    } finally { Marshal.FreeHGlobal(b); }
  }

  /// 铺满所有屏幕的包围盒（双屏拼接看视频）。记录原窗口位置到 stateFile
  public static string SpanVideo(IntPtr hwnd, string stateFile) {
    if (!IsWindow(hwnd)) return "ERR:播放器窗口不存在";
    SetProcessDpiAwarenessContext(new IntPtr(-4));
    SGRECT r;
    if (!GetWindowRect(hwnd, out r)) return "ERR:GetWindowRect 失败";
    string json = "{\"x\":" + r.Left + ",\"y\":" + r.Top + ",\"w\":" + (r.Right - r.Left) + ",\"h\":" + (r.Bottom - r.Top) + "}";
    try { System.IO.File.WriteAllText(stateFile, json); } catch { }
    IntPtr style = GetWindowLongPtr(hwnd, GWL_STYLE);
    long s = style.ToInt64();
    s &= ~((long)WS_CAPTION | (long)WS_THICKFRAME);
    s |= (long)WS_POPUP;
    SetWindowLongPtr(hwnd, GWL_STYLE, new IntPtr(s));
    int vx = GetSystemMetrics(76), vy = GetSystemMetrics(77);
    int vw = GetSystemMetrics(78), vh = GetSystemMetrics(79);
    SetWindowPos(hwnd, IntPtr.Zero, vx, vy, vw, vh, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW);
    return "OK";
  }

  /// 还原播放器窗口（优先还原 span 前位置；无记录则铺回主屏工作区）
  public static string RestoreVideo(IntPtr hwnd, string stateFile) {
    if (!IsWindow(hwnd)) return "ERR:播放器窗口不存在";
    SetProcessDpiAwarenessContext(new IntPtr(-4));
    bool have = false;
    long rx = 0, ry = 0, rw = 0, rh = 0;
    if (System.IO.File.Exists(stateFile)) {
      try {
        string txt = System.IO.File.ReadAllText(stateFile);
        var re = new System.Text.RegularExpressions.Regex("\"([xywh])\":(-?[0-9]+)");
        foreach (System.Text.RegularExpressions.Match m in re.Matches(txt)) {
          long v = long.Parse(m.Groups[2].Value);
          switch (m.Groups[1].Value) {
            case "x": rx = v; break;
            case "y": ry = v; break;
            case "w": rw = v; break;
            case "h": rh = v; break;
          }
        }
        have = rw > 0 && rh > 0;
      } catch { }
      try { System.IO.File.Delete(stateFile); } catch { }
    }
    IntPtr style = GetWindowLongPtr(hwnd, GWL_STYLE);
    long s = style.ToInt64();
    s |= (long)WS_CAPTION | (long)WS_THICKFRAME;
    s &= ~((long)WS_POPUP);
    SetWindowLongPtr(hwnd, GWL_STYLE, new IntPtr(s));
    if (have) {
      SetWindowPos(hwnd, IntPtr.Zero, (int)rx, (int)ry, (int)rw, (int)rh,
        SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW);
    } else {
      var wa = new SGRECT();
      SystemParametersInfo(SPI_GETWORKAREA, 0, ref wa, 0);
      SetWindowPos(hwnd, IntPtr.Zero, wa.Left, wa.Top, wa.Right - wa.Left, wa.Bottom - wa.Top,
        SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW);
    }
    return "OK";
  }

  // ---------------- 色彩配置（ICC）关联 ----------------
  // 走 GDI 设备上下文——实测这是 Windows 色彩引擎真正读取的路径。
  // 实测无效、已弃用的做法：WcsAssociateColorProfileWithDevice 无论设备名对错都返回 TRUE，
  // 关联却从不落地；SetICMProfileW 在权限不足时同样返回 TRUE 但不生效 → 因此必须回读校验。
  static string IccOnHandle(IntPtr hdc) {
    var sb = new StringBuilder(1024);
    uint cb = 1024;
    return GetICMProfileW(hdc, ref cb, sb) ? sb.ToString() : null;
  }

  /// 开一个**新**的设备上下文读当前关联（必须用新 DC：
  /// 同一 DC 会缓存刚 SetICMProfileW 写入的值，用它回读会得出「总是成功」的假象）
  static string IccReadFresh(string dev) {
    IntPtr hdc = CreateDCW("DISPLAY", dev, null, IntPtr.Zero);
    if (hdc == IntPtr.Zero) return null;
    try { return IccOnHandle(hdc); } finally { DeleteDC(hdc); }
  }

  /// 读某显示设备当前关联的 ICC 路径 → OK:<path> / ERR:<原因>
  public static string IccGet(string dev) {
    string p = IccReadFresh(dev);
    return p == null ? "ERR:无法打开显示设备或 GetICMProfile 无返回" : ("OK:" + p);
  }

  /// 关联 ICC 到指定显示设备 → OK / ERR:<原因>
  /// 关键：写入后**另开 DC** 回读校验才算成功
  /// （Windows 权限不足时 SetICMProfileW 照样返回 TRUE，但关联不会落到设备上）
  public static string IccSet(string dev, string profile) {
    IntPtr hdc = CreateDCW("DISPLAY", dev, null, IntPtr.Zero);
    if (hdc == IntPtr.Zero) return "ERR:无法打开显示设备";
    try { SetICMProfileW(hdc, profile); } finally { DeleteDC(hdc); }
    string now = IccReadFresh(dev);
    if (now != null && string.Equals(now, profile, StringComparison.OrdinalIgnoreCase)) return "OK";
    return "ERR:未生效(回读=" + (now == null ? "读取失败" : now) + ")";
  }

  /// 安装 ICC 到系统色彩目录（Windows 只认该目录下的配置；实测非管理员可用）
  public static string IccInstall(string profile) {
    try { return InstallColorProfileW(IntPtr.Zero, profile) ? "OK" : "ERR:InstallColorProfile 失败"; }
    catch { return "ERR:InstallColorProfile 异常"; }
  }
}

// ---------------- 系统音频端点（CoreAudio COM） ----------------
// 为什么只有 DDC 0x62 不够：0x62 只作用于「显示器内置喇叭」，笔记本内建喇叭 /
// 耳机 / 蓝牙根本没有这条通路；而部分 HDMI/DP 显示器音频端点是固定音量，
// Windows 系统滑块也调不动——所以补上「系统默认输出设备」的音量控制，两者互补。
// 接口序与 CoreAudio 官方 vtable 严格一致；接口声明省略尾部方法不影响已声明方法。
[ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
public class SGMMEnumerator { }

[Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface SGIMMEnum {
  int EnumAudioEndpoints(int flow, int mask, out SGIMMColl coll);
  int GetDefaultAudioEndpoint(int flow, int role, out SGIMMDev dev);
}

[Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface SGIMMColl {
  int GetCount(out int count);
  int Item(int index, out SGIMMDev dev);
}

[Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface SGIMMDev {
  int Activate(ref Guid iid, uint clsCtx, IntPtr actParams, [MarshalAs(UnmanagedType.IUnknown)] out object iface);
  int OpenPropertyStore(int stgm, out SGPropStore store);
  int GetId([MarshalAs(UnmanagedType.LPWStr)] out string id);
  int GetState(out int state);
}

[Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface SGPropStore {
  int GetCount(out int count);
  int GetAt(int index, out SGPKEY key);
  int GetValue(ref SGPKEY key, out SGPV value);
}

[Guid("5CDF2C82-841E-4546-9722-0CF74078229A"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface SGEndVol {
  int RegisterControlChangeNotify(IntPtr n);
  int UnregisterControlChangeNotify(IntPtr n);
  int GetChannelCount(out uint c);
  int SetMasterVolumeLevel(float db, IntPtr ctx);
  int SetMasterVolumeLevelScalar(float level, IntPtr ctx);
  int GetMasterVolumeLevel(out float db);
  int GetMasterVolumeLevelScalar(out float level);
  int SetChannelVolumeLevel(uint ch, float db, IntPtr ctx);
  int SetChannelVolumeLevelScalar(uint ch, float level, IntPtr ctx);
  int GetChannelVolumeLevel(uint ch, out float db);
  int GetChannelVolumeLevelScalar(uint ch, out float level);
  // 注意：这里必须用 int（Win32 BOOL，4 字节）。COM 互操作里 bool 默认按
  // VARIANT_BOOL（2 字节）编组，而 IAudioEndpointVolume 的静音参数是 BOOL——
  // API 写 4 字节进 2 字节缓冲会破坏栈帧，下一句托管代码报 NullReferenceException
  // （实测：AudioGet 因此全挂，AudioSet 因不调 GetMute 而幸存）。
  int SetMute(int mute, IntPtr ctx);
  int GetMute(out int mute);
  int GetVolumeStepInfo(out uint step, out uint steps);
  int VolumeStepUp(IntPtr ctx);
  int VolumeStepDown(IntPtr ctx);
  int QueryHardwareSupport(out uint mask);
  int GetVolumeRange(out float minDb, out float maxDb, out float incDb);
}

[StructLayout(LayoutKind.Sequential)]
public struct SGPKEY { public Guid fmtid; public uint pid; }

// PROPVARIANT 只消费两种：VT_LPWSTR(31) 指针 / VT_UI4(19) 内联整数。
// 关键坑：内联值放在 union 本体里，若把它当指针 Marshal.ReadInt32(v.p) =
// 按值解引用 → 访问违例，进程直接崩（PS 的 try/catch 也拦不住）。
// 必须用重叠字段原地读。
[StructLayout(LayoutKind.Explicit)]
public struct SGPV {
  [FieldOffset(0)] public ushort vt;
  [FieldOffset(8)] public IntPtr p;
  [FieldOffset(8)] public int i32;
}

public class SGAudio {
  static Guid IID_VOL = new Guid("5CDF2C82-841E-4546-9722-0CF74078229A");

  static string Esc(string s) {
    if (s == null) return "";
    return s.Replace("|", "/").Replace("\r", " ").Replace("\n", " ");
  }

  // 默认渲染端点（eRender + eMultimedia：媒体应用的默认输出，与系统托盘一致）
  static SGIMMDev DefaultDev() {
    var en = (SGIMMEnum)(object)new SGMMEnumerator();
    SGIMMDev dev;
    return en.GetDefaultAudioEndpoint(0, 1, out dev) == 0 ? dev : null;
  }

  static SGEndVol VolOf(SGIMMDev dev) {
    object o;
    if (dev.Activate(ref IID_VOL, 23, IntPtr.Zero, out o) != 0 || o == null) return null;
    return (SGEndVol)o;
  }

  /// 默认输出端点信息：OK|名称|音量0-100|静音|硬件支持掩码|形态因子(9=HDMI/DP音频)
  /// 全程分步防御：任何一步失败返回带步骤标记的 ERR，而不是笼统的 NRE
  public static string AudioGet() {
    try {
      var en = (SGIMMEnum)(object)new SGMMEnumerator();
      SGIMMDev dev;
      int hr = en.GetDefaultAudioEndpoint(0, 1, out dev);
      if (hr != 0 || dev == null) return "ERR:枚举默认设备失败 hr=" + hr;
      string name = "", ff = "";
      try {
        SGPropStore store;
        if (dev.OpenPropertyStore(0, out store) == 0 && store != null) {
          SGPV v;
          SGPKEY kn = new SGPKEY(); kn.fmtid = new Guid("a45c254e-df1c-4efd-8020-67d146a850e0"); kn.pid = 14;
          if (store.GetValue(ref kn, out v) == 0 && v.vt == 31 && v.p != IntPtr.Zero) {
            string s = Marshal.PtrToStringUni(v.p);
            if (s != null) name = s;
          }
          SGPKEY kf = new SGPKEY(); kf.fmtid = new Guid("1da5d803-d492-4edd-8c23-e0c0ffee7f0e"); kf.pid = 0;
          if (store.GetValue(ref kf, out v) == 0 && (v.vt == 19 || v.vt == 3)) ff = v.i32.ToString();
        }
      } catch (Exception) { name = name.Length > 0 ? name : "默认输出设备"; }
      object o;
      Guid iid = new Guid("5CDF2C82-841E-4546-9722-0CF74078229A");
      int ahr = dev.Activate(ref iid, 23, IntPtr.Zero, out o);
      if (ahr != 0 || o == null) return "ERR:端点激活失败 hr=" + ahr;
      SGEndVol vol = (SGEndVol)o;
      float lvl; uint mask; int mute;
      vol.GetMasterVolumeLevelScalar(out lvl);
      vol.QueryHardwareSupport(out mask);
      vol.GetMute(out mute);
      return "OK|" + Esc(name) + "|" + Math.Round(lvl * 100) + "|" + (mute != 0 ? 1 : 0) + "|" + mask + "|" + ff;
    } catch (Exception e) { return "ERR:" + Esc(e.Message); }
  }

  /// 设置系统音量 0-100（默认输出端点）
  public static string AudioSet(uint v) {
    try {
      SGIMMDev dev = DefaultDev();
      if (dev == null) return "ERR:没有默认输出设备";
      SGEndVol vol = VolOf(dev);
      if (vol == null) return "ERR:端点激活失败";
      if (v > 100) v = 100;
      return vol.SetMasterVolumeLevelScalar(v / 100f, IntPtr.Zero) == 0
        ? "OK" : "ERR:设置失败（端点可能为固定音量，常见于部分 HDMI/DP 音频）";
    } catch (Exception e) { return "ERR:" + Esc(e.Message); }
  }

  /// 系统静音开关：on 非 0 = 静音
  public static string AudioMute(uint on) {
    try {
      SGIMMDev dev = DefaultDev();
      if (dev == null) return "ERR:没有默认输出设备";
      SGEndVol vol = VolOf(dev);
      if (vol == null) return "ERR:端点激活失败";
      return vol.SetMute(on != 0 ? 1 : 0, IntPtr.Zero) == 0 ? "OK" : "ERR:静音设置失败";
    } catch (Exception e) { return "ERR:" + Esc(e.Message); }
  }
}
"#;

// ---------- uid -> devName 缓存（显示器重插后由 get_displays 刷新） ----------
/// 从显示器标识里提取「厂商+型号」段。
/// uid 形如 "MONITOR\XMI27B3\{GUID}\0007"，WMI InstanceName 形如
/// "DISPLAY\XMI27B3\5&...&UID4352"，PerMonitorSettings 键名形如 "XMI27B30_0A_..."。
/// 三者前缀不同，统一取第二段对齐（用于名字匹配与 DPI 档匹配）。
fn short_id(s: &str) -> String {
    s.split('\\').nth(1).unwrap_or(s).to_string()
}

fn dev_cache() -> &'static Mutex<Option<HashMap<String, String>>> {
    static CACHE: OnceLock<Mutex<Option<HashMap<String, String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// uid（如 DISPLAY\XMI27B3\5&...&UID4352）→ 设备名（\\.\DISPLAY1）
/// 缓存未命中时重新枚举
fn resolve_dev(uid: &str) -> Result<String, String> {
    {
        let guard = dev_cache().lock().unwrap();
        if let Some(map) = guard.as_ref() {
            if let Some(dev) = map.get(uid) {
                return Ok(dev.clone());
            }
        }
    }
    // 重新枚举填充
    let raw = ps_core("[SGCore]::ListDisplays()")?;
    let mut map: HashMap<String, String> = HashMap::new();
    for line in raw.lines() {
        let p: Vec<&str> = line.split('|').collect();
        if p.len() >= 3 && p[0] == "D" {
            map.insert(p[2].to_string(), p[1].to_string());
        }
    }
    if map.is_empty() {
        return Err("未枚举到显示器".to_string());
    }
    let dev = map
        .get(uid)
        .cloned()
        .ok_or_else(|| format!("未找到显示器 {}", uid))?;
    *dev_cache().lock().unwrap() = Some(map);
    Ok(dev)
}

// ---------- 显示器列表 ----------
pub fn get_displays() -> Vec<DisplayInfo> {
    let script = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\nAdd-Type -TypeDefinition '{cs}';\n$lines = [SGCore]::ListDisplays() -split ([char]10);\n$lines | ForEach-Object {{ Write-Output $_ }};\n$devs = @();\nforeach ($l in $lines) {{ $p = $l -split '\\|'; if ($p[0] -eq 'D') {{ $devs += $p[1] }} }};\n[SGCore]::DisplayNameMap();\nGet-CimInstance -Namespace root/wmi -ClassName WmiMonitorID -ErrorAction SilentlyContinue | ForEach-Object {{\n  $nm = (($_.UserFriendlyName | Where-Object {{ [int]$_ -ne 0 }} | ForEach-Object {{ [char][int]$_ }}) -join '');\n  $inst = $_.InstanceName -replace '_\\d+$', '';\n  if ($_.Active -and $nm -and $nm.Trim()) {{ Write-Output ('N|' + $inst + '|' + $nm) }}\n}};\nforeach ($dev in $devs) {{ $pr = [SGCore]::DDCProbe($dev); $b = ''; $v = ''; if ($pr -eq '1') {{ $b = [SGCore]::DDCRead($dev, [byte]0x10); $v = [SGCore]::DDCRead($dev, [byte]0x62) }}; Write-Output ('P|' + $dev + '|' + $pr + '|' + $b + '|' + $v); Write-Output ('C|' + $dev + '|' + [SGCore]::IccGet($dev)) }};\nGet-CimInstance -Namespace root/wmi -ClassName WmiMonitorBrightness -ErrorAction SilentlyContinue | ForEach-Object {{ $sg = ($_.InstanceName -split '\\\\')[1]; if ($sg) {{ Write-Output ('W|' + $sg + '|' + $_.CurrentBrightness) }} }}",
        cs = CORE_CS
    );
    let raw = ps(&script).unwrap_or_default();
    let mut rows: Vec<(String, String, u32, u32, u32, bool, u32)> = Vec::new(); // dev,uid,w,h,hz,main,ori
    let mut dc_names: HashMap<String, String> = HashMap::new(); // DisplayConfig: dev(小写) → 友好名
    let mut names: HashMap<String, String> = HashMap::new();
    let mut ddc: HashMap<String, bool> = HashMap::new();
    let mut bri: HashMap<String, u32> = HashMap::new();
    let mut vol: HashMap<String, u32> = HashMap::new();
    let mut icc: HashMap<String, String> = HashMap::new();
    // WMI 亮度（笔记本内屏：无 DDC/CI，走 root\wmi 的 ACPI 亮度）键 = uid 第二段
    let mut wmi_bri: HashMap<String, u32> = HashMap::new();
    for line in raw.lines() {
        let p: Vec<&str> = line.split('|').collect();
        if p.is_empty() {
            continue;
        }
        match p[0] {
            "D" if p.len() >= 8 => {
                let w: u32 = p[3].trim().parse().unwrap_or(0);
                let h: u32 = p[4].trim().parse().unwrap_or(0);
                let hz: u32 = p[5].trim().parse().unwrap_or(0);
                let main = p[6].trim() == "1";
                let ori: u32 = p[7].trim().parse().unwrap_or(0);
                rows.push((p[1].to_string(), p[2].to_string(), w, h, hz, main, ori));
            }
            "N" if p.len() >= 3 => {
                names.insert(short_id(p[1]), p[2].to_string());
            }
            // M|\\.\DISPLAYn|<友好名> —— DisplayConfig 权威映射（与驱动栈状态无关）
            "M" if p.len() >= 3 => {
                dc_names.insert(p[1].trim().to_lowercase(), p[2].to_string());
            }
            "P" if p.len() >= 3 => {
                let has = p[2].trim() == "1";
                ddc.insert(p[1].to_string(), has);
                if has {
                    // P|dev|1|亮度|音量（探测时顺带读回真实值，避免 UI 滑块初值错误）
                    if let Some(v) = p.get(3).and_then(|s| s.trim().parse::<u32>().ok()) {
                        bri.insert(p[1].to_string(), v);
                    }
                    if let Some(v) = p.get(4).and_then(|s| s.trim().parse::<u32>().ok()) {
                        vol.insert(p[1].to_string(), v);
                    }
                }
            }
            // C|dev|OK:<完整 ICC 路径> —— UI 只展示文件名
            "C" if p.len() >= 3 => {
                if let Some(full) = p[2].trim().strip_prefix("OK:") {
                    if let Some(base) = full.rsplit('\\').next() {
                        if !base.is_empty() {
                            icc.insert(p[1].to_string(), base.to_string());
                        }
                    }
                }
            }
            // W|<厂商型号段>|<当前亮度> —— WMI 内屏亮度（无 DDC 的显示器的回退来源）
            "W" if p.len() >= 3 => {
                if let Ok(v) = p[2].trim().parse::<u32>() {
                    wmi_bri.insert(p[1].to_string(), v);
                }
            }
            _ => {}
        }
    }
    // 刷新 dev 缓存
    {
        let mut map: HashMap<String, String> = HashMap::new();
        for (dev, uid, _, _, _, _, _) in &rows {
            map.insert(uid.clone(), dev.clone());
        }
        if !map.is_empty() {
            *dev_cache().lock().unwrap() = Some(map);
        }
    }
    // 名字匹配（优先级从高到低）：
    //  ① DisplayConfig 权威映射（GDI 设备名 → 友好名）：与 PnP/WMI 监视器状态无关，
    //     Win11 24H2 上那两者失效时它是唯一可靠来源
    //  ② uid 第二段（厂商+型号）对齐 WMI 名字（原有逻辑）
    //  ③ 1↔1 兜底：仅当 ② 恰好用掉 N-1 个名字且恰剩一行未配时才猜（两侧都不唯一时不猜）
    let mut matched: Vec<Option<String>> = Vec::with_capacity(rows.len());
    let mut seg_used = 0usize;
    for (dev, uid, _, _, _, _, _) in rows.iter() {
        let mut name = dc_names
            .get(&dev.to_lowercase())
            .cloned()
            .filter(|n| !n.is_empty());
        if name.is_none() {
            let key = short_id(uid);
            if !key.is_empty() {
                name = names.get(&key).cloned().filter(|n| !n.is_empty());
                if name.is_some() {
                    seg_used += 1;
                }
            }
        }
        matched.push(name);
    }
    {
        let unmatched: Vec<usize> = matched
            .iter()
            .enumerate()
            .filter(|(_, m)| m.is_none())
            .map(|(i, _)| i)
            .collect();
        if unmatched.len() == 1 && names.len() == seg_used + 1 {
            if let Some(n) = names.values().next() {
                matched[unmatched[0]] = Some(n.clone());
            }
        }
    }
    let mut result: Vec<DisplayInfo> = Vec::new();
    for (i, (dev, uid, w, h, hz, main, ori)) in rows.into_iter().enumerate() {
        let name = matched
            .get(i)
            .and_then(|m| m.clone())
            .unwrap_or_else(|| format!("显示器 {}", result.len() + 1));
        let res = if w > 0 && h > 0 {
            format!("{}x{}", w, h)
        } else {
            String::new()
        };
        let _ = ori; // 当前方向信息暂不暴露
        result.push(DisplayInfo {
            id: uid.clone(),
            name,
            resolution: res,
            hz: if hz > 0 { Some(hz) } else { None },
            main,
            connected: true,
            ddc: ddc.get(&dev).copied().unwrap_or(false),
            brightness: bri.get(&dev).copied().or_else(|| wmi_bri.get(&short_id(&uid)).copied()),
            volume: vol.get(&dev).copied(),
            color_profile: icc.get(&dev).cloned(),
            ddc_id: None,
        });
    }
    result
}

// ---------- DDC：亮度 / 音量（按显示器精确控制） ----------
fn vcp_op(display_id: &str, code: u8, value: Option<u32>) -> Result<(), String> {
    let dev = resolve_dev(display_id)?;
    let call = match value {
        Some(v) => format!(
            "[SGCore]::DDCWrite('{dev}',[byte]{code},[uint32]{v})",
            dev = dev,
            code = code,
            v = v
        ),
        None => format!("[SGCore]::DDCRead('{dev}',[byte]{code})", dev = dev, code = code),
    };
    let out = ps_core(&call)?;
    let t = out.trim();
    if t == "OK" {
        Ok(())
    } else if t == "ERR" {
        Err("该显示器不支持 DDC/CI".to_string())
    } else if t.starts_with("ERR:") {
        Err(t[4..].to_string())
    } else {
        // 读值场景：直接返回数字不适用，这里仅用于写
        Err("DDC 操作失败".to_string())
    }
}

pub fn set_brightness(display_id: &str, value: u32) -> Result<(), String> {
    let v = value.clamp(0, 100);
    if vcp_op(display_id, 0x10, Some(v)).is_ok() {
        return Ok(());
    }
    // DDC 失败的典型场景：笔记本内屏（eDP 面板根本没有 DDC/CI 通路）。
    // 回退 WMI 亮度（ACPI _BCM，内屏的标准控制方式，与系统亮度滑块同源）。
    let seg = short_id(display_id);
    if !seg.is_empty() && seg.chars().all(|c| c.is_ascii_alphanumeric() || "_- &{}".contains(c)) {
        let script = format!(
            "$b = Get-CimInstance -Namespace root/wmi -ClassName WmiMonitorBrightnessMethods -ErrorAction SilentlyContinue | Where-Object {{ ($_.InstanceName -split '\\\\')[1] -eq '{seg}' }} | Select-Object -First 1; if ($b) {{ Invoke-CimMethod -InputObject $b -MethodName WmiSetBrightness -Arguments @{{Timeout=0; Brightness={v}}} | Out-Null; Write-Output OK }} else {{ Write-Output 'ERR:no-wmi-brightness' }}",
            seg = seg,
            v = v
        );
        let out = ps(&script)?;
        if out.trim() == "OK" {
            return Ok(());
        }
        return Err(format!(
            "此屏不支持 DDC/CI 且 WMI 亮度不可用（{}）。外接屏请在显示器 OSD 菜单开启 DDC/CI；内屏亮度失败常见于驱动栈异常，可按 Win+Ctrl+Shift+B 重启图形驱动后重试",
            out.trim().trim_start_matches("ERR:").trim()
        ));
    }
    Err("该显示器不支持 DDC/CI".to_string())
}

pub fn set_volume(display_id: &str, value: u32) -> Result<(), String> {
    let v = value.clamp(0, 100);
    vcp_op(display_id, 0x62, Some(v))
}

// ---------- 系统音量（CoreAudio 默认输出端点） ----------

/// 系统默认输出设备（eMultimedia 角色，与托盘音量一致）
#[derive(Serialize, Clone, Debug)]
pub struct SystemAudio {
    /// 端点友好名（如「扬声器 (2- Realtek(R) Audio)」「NE160QDM-NZ8 (NVIDIA High Definition Audio)」）
    pub name: String,
    /// 当前音量 0-100
    pub volume: u32,
    /// 是否静音
    pub mute: bool,
    /// false = 固定音量端点：Windows 滑块也无效（部分 HDMI/DP 音频如此），
    /// 此时显示器喇叭音量请用对应显示器的 DDC 0x62 滑块
    pub adjustable: bool,
    /// 端点形态因子（9 = DigitalAudioDisplayDevice，即 HDMI/DP 显示器音频）
    pub form_factor: u32,
}

pub fn get_system_audio() -> Result<SystemAudio, String> {
    let out = ps_core("[SGAudio]::AudioGet()")?;
    let t = out.trim();
    if let Some(rest) = t.strip_prefix("OK|") {
        let p: Vec<&str> = rest.split('|').collect();
        if p.len() >= 5 {
            return Ok(SystemAudio {
                name: p[0].to_string(),
                volume: p[1].parse().unwrap_or(0),
                mute: p[2] == "1",
                adjustable: p[3].parse::<u32>().unwrap_or(0) & 1 != 0,
                form_factor: p[4].parse().unwrap_or(0),
            });
        }
    }
    Err(t.strip_prefix("ERR:").unwrap_or(t).to_string())
}

pub fn set_system_volume(v: u32) -> Result<(), String> {
    let out = ps_core(&format!("[SGAudio]::AudioSet([uint32]{})", v.clamp(0, 100)))?;
    let t = out.trim();
    if t == "OK" {
        Ok(())
    } else {
        Err(t.strip_prefix("ERR:").unwrap_or(t).to_string())
    }
}

pub fn set_system_mute(on: bool) -> Result<(), String> {
    let out = ps_core(&format!("[SGAudio]::AudioMute([uint32]{})", if on { 1 } else { 0 }))?;
    let t = out.trim();
    if t == "OK" {
        Ok(())
    } else {
        Err(t.strip_prefix("ERR:").unwrap_or(t).to_string())
    }
}
// ---------- 色彩同步（ICC 关联） ----------
//
// 以下实现方式由本机（Win11 24H2 双屏、非管理员）实测确定：
//   ① WcsAssociateColorProfileWithDevice：设备名无论对错都返回 TRUE，关联却从不落地 → 已弃用
//   ② GetICMProfileW / SetICMProfileW（GDI 设备上下文）才是色彩引擎真正读取的路径，读取可靠
//   ③ 非管理员下 SetICMProfileW 同样返回 TRUE 却**不生效** → 必须回读校验（校验在 C# 侧 IccSet 内）
//   ④ InstallColorProfileW 非管理员可用，可把用户自备的 P3 / AdobeRGB 装进系统色彩目录
const COLOR_DIR: &str = "screenguard_icc";
/// 系统色彩目录：Windows 只认这里的配置文件
const SYSTEM_COLOR_DIR: &str = r"C:\Windows\System32\spool\drivers\color";

/// 定位目标 ICC 文件。
/// sRGB 用系统内置；Display P3 / Adobe RGB 系统不带，需用户自备放到
/// %LOCALAPPDATA%\screenguard_icc\，由 apply_color_space 自动安装进系统色彩目录。
fn icc_path(space: &str) -> Result<String, String> {
    match space.to_lowercase().as_str() {
        "srgb" => {
            let p = format!(r"{}\sRGB Color Space Profile.icm", SYSTEM_COLOR_DIR);
            if std::path::Path::new(&p).exists() {
                Ok(p)
            } else {
                Err("未找到系统内置 sRGB 配置（sRGB Color Space Profile.icm）".to_string())
            }
        }
        "p3" | "adobergb" => {
            let dir = format!("{}\\{}", std::env::var("LOCALAPPDATA").unwrap_or_default(), COLOR_DIR);
            let file = if space.eq_ignore_ascii_case("p3") {
                "DisplayP3.icc"
            } else {
                "AdobeRGB1998.icc"
            };
            let path = format!("{}\\{}", dir, file);
            if std::path::Path::new(&path).exists() {
                Ok(path)
            } else {
                Err(format!(
                    "缺少色彩配置文件 {}：请先把它放到 {}（Windows 不自带 Display P3 / Adobe RGB；macOS 上可从 /System/Library/ColorSync/Profiles/ 复制）",
                    file, dir
                ))
            }
        }
        _ => Err("未知色彩空间".to_string()),
    }
}

/// 在已加载 CORE_CS 的会话里执行一段桥接调用
fn ps_core(call: &str) -> Result<String, String> {
    let script = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\nAdd-Type -TypeDefinition '{cs}';\n{call}",
        cs = CORE_CS,
        call = call
    );
    ps(&script)
}

/// 所有显示屏的设备名（\\.\DISPLAYn，即 CreateDC / EnumDisplaySettings 用的名字）
fn display_devs() -> Result<Vec<String>, String> {
    let raw = ps_core("[SGCore]::ListDisplays()")?;
    let devs: Vec<String> = raw
        .lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split('|').collect();
            if p.len() >= 8 && p[0] == "D" && !p[1].is_empty() {
                Some(p[1].to_string())
            } else {
                None
            }
        })
        .collect();
    if devs.is_empty() {
        Err("未枚举到显示器".to_string())
    } else {
        Ok(devs)
    }
}

/// 读某设备当前关联的 ICC 完整路径
fn icc_get(dev: &str) -> Option<String> {
    let out = ps_core(&format!("[SGCore]::IccGet('{d}')", d = dev)).ok()?;
    out.trim()
        .strip_prefix("OK:")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 安装 ICC 到系统色彩目录
fn icc_install(profile: &str) -> Result<(), String> {
    let out = ps_core(&format!("[SGCore]::IccInstall('{p}')", p = profile))?;
    let t = out.trim();
    if t == "OK" {
        Ok(())
    } else if let Some(e) = t.strip_prefix("ERR:") {
        Err(e.to_string())
    } else {
        Err(t.to_string())
    }
}

/// 关联 ICC（含回读校验，校验在 C# 侧完成）
fn icc_set(dev: &str, profile: &str) -> Result<(), String> {
    let out = ps_core(&format!("[SGCore]::IccSet('{d}','{p}')", d = dev, p = profile))?;
    let t = out.trim();
    if t == "OK" {
        Ok(())
    } else if let Some(e) = t.strip_prefix("ERR:") {
        Err(e.to_string())
    } else {
        Err(t.to_string())
    }
}

/// 当前进程是否已提权（whoami /groups 的完整性级别：S-1-16-12288 = 高完整性 = 管理员）
fn is_elevated() -> bool {
    run_cmd("whoami", &["/groups"])
        .map(|o| o.contains("S-1-16-12288"))
        .unwrap_or(false)
}

/// 把指定 ICC 关联到所有显示屏；至少一台成功即算成功，全部失败则给出可操作的原因
fn associate_all(profile: &str) -> Result<(), String> {
    let devs = display_devs()?;
    let mut ok = 0usize;
    let mut errs: Vec<String> = Vec::new();
    for d in &devs {
        match icc_set(d, profile) {
            Ok(()) => ok += 1,
            Err(e) => errs.push(format!("{} {}", d, e)),
        }
    }
    if ok > 0 {
        return Ok(());
    }
    let hint = if is_elevated() {
        "请在「设置 → 系统 → 显示 → 高级显示 → 颜色管理」中指定配置文件".to_string()
    } else {
        "Windows 变更显示器色彩配置需要管理员权限：请右键本程序选「以管理员身份运行」".to_string()
    };
    Err(format!(
        "色彩关联未生效（{} 台设备全部失败）→ {}{}",
        devs.len(),
        hint,
        if errs.is_empty() { String::new() } else { format!("（{}）", errs.join("；")) }
    ))
}

/// Windows 端「色彩同步」：把 ICC 关联到所有显示屏。
/// 关联可随时改回、无破坏性；失败会给出真实原因，不再静默假装成功。
pub fn apply_color_space(space: &str) -> Result<(), String> {
    let profile = icc_path(space)?;
    // 先安装进系统色彩目录（Windows 只认该目录下的配置）；sRGB 为系统自带，装不上可忽略
    if let Err(e) = icc_install(&profile) {
        if !profile.starts_with(SYSTEM_COLOR_DIR) {
            return Err(format!("无法安装色彩配置：{}", e));
        }
    }
    associate_all(&profile)
}

/// 「对齐 Mac 内建屏」在 Windows 没有对应物（Windows 无内建 P3 屏），
/// 因此语义改为：以主屏当前关联的 ICC 为准，把其余屏全部对齐到主屏。
pub fn match_mac() -> Result<(), String> {
    let displays = get_displays();
    if displays.len() < 2 {
        return Err("需要主屏 + 副屏各一块才能使用此功能".to_string());
    }
    let main = displays.iter().find(|d| d.main).ok_or("未找到主屏")?;
    let dev = resolve_dev(&main.id)?;
    let profile = icc_get(&dev).ok_or("读取主屏当前色彩配置失败（GetICMProfile 无返回）")?;
    associate_all(&profile)
}

/// 匹配两屏 PPI：检查主/副屏 DPI 缩放是否一致（窗口跨屏等大的前提）。
/// Windows 的缩放只能通过系统设置修改（需注销），此函数做诊断并给出明确指引，不擅自改动。
pub fn match_ppi() -> Result<(), String> {
    let displays = get_displays();
    if displays.len() < 2 {
        return Err("需要主屏 + 副屏各一块才能使用此功能".to_string());
    }
    let main = displays.iter().find(|d| d.main).ok_or("未找到主屏")?;
    let sec = displays
        .iter()
        .find(|d| !d.main)
        .ok_or("需要主屏 + 副屏各一块才能使用此功能")?;
    // 逻辑尺寸 = 物理 / 缩放；这里用注册表 PerMonitorSettings 读取每屏的 DPI 档
    let script = "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\n\
                  $vals = Get-ChildItem 'HKCU:\\Control Panel\\Desktop\\PerMonitorSettings' -ErrorAction SilentlyContinue;\n\
                  foreach ($v in $vals) { Write-Output ('S|' + $v.PSChildName + '|' + (Get-ItemProperty $v.PSPath -ErrorAction SilentlyContinue).DpiValue) }";
    let raw = ps(script).unwrap_or_default();
    let mut dpi_main: Option<u32> = None;
    let mut dpi_sec: Option<u32> = None;
    // PerMonitorSettings 键名形如 "XMI27B30_0A_07EA_47^HASH"（厂商型号开头），
    // 用 uid 的第二段(厂商+型号)做前缀匹配；同一屏可能有多条历史键，取首个命中。
    let main_key = short_id(&main.id).to_lowercase();
    let sec_key = short_id(&sec.id).to_lowercase();
    for line in raw.lines() {
        let p: Vec<&str> = line.split('|').collect();
        if p.len() >= 3 && p[0] == "S" {
            let key = p[1].to_lowercase();
            let val: u32 = p[2].trim().parse().unwrap_or(0);
            if !main_key.is_empty() && key.starts_with(&main_key) && dpi_main.is_none() {
                dpi_main = Some(val);
            }
            if !sec_key.is_empty() && key.starts_with(&sec_key) && dpi_sec.is_none() {
                dpi_sec = Some(val);
            }
        }
    }
    // PerMonitorSettings 的键名与 EDID 相关，匹配不上时用系统默认（0 代表跟随系统）
    let m = dpi_main.unwrap_or(0);
    let s = dpi_sec.unwrap_or(0);
    if m == s || (m == 0 && s == 0) {
        Ok(())
    } else {
        Err(format!(
            "主屏缩放档 {} 与副屏 {} 不一致。请在「设置 → 系统 → 屏幕 → 缩放」把两块屏设为相同百分比后重试（Windows 需注销登录生效）",
            if m == 0 { "系统默认".to_string() } else { m.to_string() },
            if s == 0 { "系统默认".to_string() } else { s.to_string() }
        ))
    }
}

// ---------- 副屏旋转 ----------
/// 副屏 = 第一块非主屏
fn secondary_dev() -> Result<(String, u32), String> {
    let raw = ps_core("[SGCore]::ListDisplays()")?;
    let mut main_dev: Option<String> = None;
    let mut sec: Option<(String, u32)> = None;
    for line in raw.lines() {
        let p: Vec<&str> = line.split('|').collect();
        if p.len() >= 8 && p[0] == "D" {
            let ori: u32 = p[7].trim().parse().unwrap_or(0);
            if p[6].trim() == "1" {
                main_dev = Some(p[1].to_string());
            } else if sec.is_none() && main_dev.is_some() {
                sec = Some((p[1].to_string(), ori));
            } else if sec.is_none() && main_dev.is_none() {
                sec = Some((p[1].to_string(), ori));
            }
        }
    }
    sec.ok_or_else(|| "需要主屏 + 副屏各一块才能使用此功能".to_string())
}

/// 横竖屏切换（再点一次转回）
pub fn rotate_secondary() -> Result<(), String> {
    let (dev, ori) = secondary_dev()?;
    let target = if ori == 1 || ori == 3 { 0 } else { 3 };
    let call = format!(
        "[SGCore]::Rotate('{dev}',[uint32]{target})",
        dev = dev,
        target = target
    );
    let out = ps_core(&call)?;
    let t = out.trim();
    if t == "OK" {
        Ok(())
    } else if t.starts_with("ERR:") {
        Err(t[4..].to_string())
    } else {
        Err(t.to_string())
    }
}

/// 恢复副屏为竖屏（安全复位）
pub fn restore_secondary() -> Result<(), String> {
    let (dev, ori) = secondary_dev()?;
    if ori == 3 || ori == 1 {
        return Ok(());
    }
    let call = format!("[SGCore]::Rotate('{dev}',[uint32]3)", dev = dev);
    let out = ps_core(&call)?;
    let t = out.trim();
    if t == "OK" {
        Ok(())
    } else if t.starts_with("ERR:") {
        Err(t[4..].to_string())
    } else {
        Err(t.to_string())
    }
}

// ---------- 双屏铺满（PotPlayer / VLC） ----------
fn state_file() -> String {
    let tmp = std::env::var("TEMP").unwrap_or_else(|_| "C:\\Windows\\Temp".to_string());
    format!("{}\\screenguard_span.json", tmp)
}

/// 找播放器进程句柄；未运行则尝试启动 PotPlayer（其次 VLC）
fn player_hwnd() -> Result<i64, String> {
    // 1) 已运行的播放器
    let probe = "Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -match 'PotPlayerMini|PotPlayer|vlc|mpc-hc|mpv' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1 -ExpandProperty MainWindowHandle";
    if let Ok(out) = ps(probe) {
        let t = out.trim();
        if !t.is_empty() {
            if let Ok(h) = t.parse::<i64>() {
                return Ok(h);
            }
        }
    }
    // 2) 依次尝试启动 PotPlayer / VLC：某个没起来就继续试下一个
    //    （此前无论成败都无条件 break，装了 PotPlayer 但启动慢时会直接失败，不会回退 VLC）
    let candidates = [
        "C:\\Program Files\\DAUM\\PotPlayer\\PotPlayerMini64.exe",
        "C:\\Program Files (x86)\\DAUM\\PotPlayer\\PotPlayerMini.exe",
        "C:\\Program Files\\VideoLAN\\VLC\\vlc.exe",
        "C:\\Program Files (x86)\\VideoLAN\\VLC\\vlc.exe",
    ];
    for path in candidates.iter() {
        if !std::path::Path::new(path).exists() {
            continue;
        }
        let _ = std::process::Command::new(path).spawn();
        // 最多等 6 秒（0.5s × 12）：窗口一出现立即返回，不必等满
        for _ in 0..12 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if let Ok(out) = ps(probe) {
                let t = out.trim();
                if let Ok(h) = t.parse::<i64>() {
                    return Ok(h);
                }
            }
        }
    }
    Err("未找到播放器：请先手动打开 PotPlayer 或 VLC（双屏铺满依赖播放器窗口）".to_string())
}

/// 双屏铺满：播放器窗口拉伸到所有屏幕的包围盒
pub fn span_video() -> Result<(), String> {
    let hwnd = player_hwnd()?;
    let call = format!(
        "[SGCore]::SpanVideo([IntPtr]{hwnd}, '{sf}')",
        hwnd = hwnd,
        sf = state_file()
    );
    let out = ps_core(&call)?;
    let t = out.trim();
    if t == "OK" {
        Ok(())
    } else if t.starts_with("ERR:") {
        Err(t[4..].to_string())
    } else {
        Err(t.to_string())
    }
}

/// 恢复播放器窗口
pub fn restore_video() -> Result<(), String> {
    let hwnd = player_hwnd()?;
    let call = format!(
        "[SGCore]::RestoreVideo([IntPtr]{hwnd}, '{sf}')",
        hwnd = hwnd,
        sf = state_file()
    );
    let out = ps_core(&call)?;
    let t = out.trim();
    if t == "OK" {
        Ok(())
    } else if t.starts_with("ERR:") {
        Err(t[4..].to_string())
    } else {
        Err(t.to_string())
    }
}
