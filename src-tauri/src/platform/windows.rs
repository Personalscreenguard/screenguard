//! Windows 平台实现：全部通过 PowerShell + 内嵌 C# (Win32 P/Invoke) 落地，
//! 延续项目「零额外 Rust 依赖」的风格。每个命令独立 powershell 进程，
//! C# 代码用单引号包裹传入 Add-Type（C# 内不含单引号字符）。

use super::{run_cmd, AudioEndpoint, BatteryInfo, DisplayInfo, SystemAudio};
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

  // ---- 软件层总亮度（gamma ramp）：不依赖 DDC/CI，对任何屏都生效 ----
  [DllImport("gdi32.dll")] static extern bool SetDeviceGammaRamp(IntPtr hdc, ushort[] ramp);
  [DllImport("gdi32.dll")] static extern bool GetDeviceGammaRamp(IntPtr hdc, ushort[] ramp);
  // ---- 前台窗口铺满 / 恢复（不限于播放器）----
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern IntPtr GetShellWindow();
  [DllImport("user32.dll")] static extern IntPtr GetDesktopWindow();
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int max);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
  // ---- HDR（advanced color）状态读写 ----
  [DllImport("user32.dll")] static extern int DisplayConfigGetDeviceInfo(ref SGDC_ACI pkt);
  [DllImport("user32.dll")] static extern int DisplayConfigGetDeviceInfo(ref SGDC_ACS pkt);
  [DllImport("user32.dll")] static extern int DisplayConfigSetDeviceInfo(ref SGDC_ACS pkt);
  [DllImport("user32.dll", EntryPoint = "SetDisplayConfig")] static extern int SetDisplayConfigApply(uint np, IntPtr p, uint nm, IntPtr m, uint flags);
  [DllImport("kernel32.dll")] static extern uint GetCurrentProcessId();
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumWinProc cb, IntPtr p);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
  public delegate bool EnumWinProc(IntPtr h, IntPtr p);

  /// DISPLAYCONFIG_DEVICE_INFO_HEADER（20 字节；LUID 拆两个 32 位，避免 .NET 对齐补白）
  [StructLayout(LayoutKind.Sequential)]
  struct SGDC_HDR2 { public uint type; public uint size; public uint adapterLow; public int adapterHigh; public uint id; }
  /// DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO（32 字节）。value 位 0=支持, 位 1=已启用
  [StructLayout(LayoutKind.Sequential)]
  struct SGDC_ACI { public SGDC_HDR2 header; public uint value; public uint colorEncoding; public uint bits; }
  /// DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE（24 字节）
  [StructLayout(LayoutKind.Sequential)]
  struct SGDC_ACS { public SGDC_HDR2 header; public uint enable; }

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

  // ===== 软件层总亮度（gamma ramp）：不依赖 DDC/CI，对任何显示器都生效 =====
  // 设计要点：三通道写同一条曲线 → 只改明暗、不动色相（颜色安全）；
  // 范围锁 40~160，避免把屏幕调到看不见；gamma ramp 不写注册表、重启即失效（天然可回滚）。

  /// 生成亮度曲线：out = v^(100/pct)，端点 0/1 保持不变（不削顶、不发灰）
  static ushort[] BuildRamp(int pct) {
    if (pct < 40) pct = 40;
    if (pct > 160) pct = 160;
    double inv = 100.0 / pct;
    var ramp = new ushort[768];
    for (int i = 0; i < 256; i++) {
      double v = i / 255.0;
      double o = Math.Pow(v, inv);
      int w = (int)Math.Round(o * 65535.0);
      if (w < 0) w = 0;
      if (w > 65535) w = 65535;
      ushort u = (ushort)w;
      ramp[i] = u; ramp[i + 256] = u; ramp[i + 512] = u;
    }
    return ramp;
  }

  /// 把总亮度应用到所有在用屏幕。返回逐屏结果：OK|<dev> / FAIL|<dev>
  public static string GammaSetAll(int pct) {
    if (pct < 40) pct = 40;
    if (pct > 160) pct = 160;
    ushort[] ramp = BuildRamp(pct);
    var sb = new StringBuilder();
    for (int i = 1; i <= 12; i++) {
      string dev = "\\\\.\\DISPLAY" + i;
      var dm = AllocDevMode();
      try {
        if (!EnumDisplaySettingsW(dev, -1, dm)) continue;
        if (Marshal.ReadInt32(dm, OFF_PELSWIDTH) <= 0) continue;
        IntPtr dc = CreateDCW(null, dev, null, IntPtr.Zero);
        if (dc == IntPtr.Zero) continue;
        try {
          bool ok = SetDeviceGammaRamp(dc, ramp);
          sb.Append(ok ? "OK|" : "FAIL|").Append(dev).Append((char)10);
        } finally { DeleteDC(dc); }
      } finally { Marshal.FreeHGlobal(dm); }
    }
    return sb.ToString();
  }

  /// 读回当前总亮度（按中灰点反解 gamma 指数）。行: G|<dev>|<pct>
  public static string GammaGetAll() {
    var sb = new StringBuilder();
    for (int i = 1; i <= 12; i++) {
      string dev = "\\\\.\\DISPLAY" + i;
      var dm = AllocDevMode();
      try {
        if (!EnumDisplaySettingsW(dev, -1, dm)) continue;
        if (Marshal.ReadInt32(dm, OFF_PELSWIDTH) <= 0) continue;
        IntPtr dc = CreateDCW(null, dev, null, IntPtr.Zero);
        if (dc == IntPtr.Zero) continue;
        try {
          var ramp = new ushort[768];
          int pct = 100;
          if (GetDeviceGammaRamp(dc, ramp)) {
            double mid = ramp[128] / 65535.0;          // 128/255 = 0.50196
            if (mid > 0.03 && mid < 0.97) {
              double f = Math.Log(0.50196) / Math.Log(mid);
              if (f > 0.3 && f < 3.5) pct = (int)Math.Round(f * 100.0);
            }
          }
          sb.Append("G|").Append(dev).Append("|").Append(pct).Append((char)10);
        } finally { DeleteDC(dc); }
      } finally { Marshal.FreeHGlobal(dm); }
    }
    return sb.ToString();
  }

  // ===== HDR（advanced color）状态读写 =====
  /// 每台屏的 HDR 状态：H|<dev>|<是否支持>|<是否已启用>
  public static string HDRStates() {
    var sb = new StringBuilder();
    uint nPaths, nModes;
    if (GetDisplayConfigBufferSizes(2, out nPaths, out nModes) != 0 || nPaths == 0) return "";
    IntPtr pa = Marshal.AllocHGlobal((int)(nPaths * 72));
    IntPtr ma = Marshal.AllocHGlobal((int)(nModes * 64));
    try {
      uint p = nPaths, m = nModes;
      if (QueryDisplayConfigRaw(2, ref p, pa, ref m, ma, IntPtr.Zero) != 0) return "";
      var seen = new System.Collections.Generic.HashSet<string>();
      for (int i = 0; i < p; i++) {
        int off = i * 72;
        var src = new SGDC_SRC_NAME();
        src.header.type = 1; src.header.size = (uint)Marshal.SizeOf(typeof(SGDC_SRC_NAME));
        src.header.adapterId.LowPart = (uint)Marshal.ReadInt32(pa, off);
        src.header.adapterId.HighPart = Marshal.ReadInt32(pa, off + 4);
        src.header.id = (uint)Marshal.ReadInt32(pa, off + 8);
        if (DisplayConfigGetDeviceInfo(ref src) != 0) continue;
        string dev = src.viewGdiDeviceName;
        if (string.IsNullOrEmpty(dev) || !seen.Add(dev.ToUpperInvariant())) continue;
        var aci = new SGDC_ACI();
        aci.header.type = 9;                          // GET_ADVANCED_COLOR_INFO
        aci.header.size = (uint)Marshal.SizeOf(typeof(SGDC_ACI));
        aci.header.adapterLow = (uint)Marshal.ReadInt32(pa, off + 20);
        aci.header.adapterHigh = Marshal.ReadInt32(pa, off + 24);
        aci.header.id = (uint)Marshal.ReadInt32(pa, off + 28);
        if (DisplayConfigGetDeviceInfo(ref aci) != 0) continue;
        int sup = (int)(aci.value & 1);
        int en = (int)((aci.value >> 1) & 1);
        sb.Append("H|").Append(dev).Append("|").Append(sup).Append("|").Append(en).Append((char)10);
      }
      return sb.ToString();
    } finally { Marshal.FreeHGlobal(pa); Marshal.FreeHGlobal(ma); }
  }

  /// 把所有支持 HDR 的屏统一设为开(on=1)或关(on=0)。返回逐屏结果 R|dev|OK或ERR
  public static string HDRSetAll(int on) {
    var sb = new StringBuilder();
    uint nPaths, nModes;
    if (GetDisplayConfigBufferSizes(2, out nPaths, out nModes) != 0 || nPaths == 0) return "";
    IntPtr pa = Marshal.AllocHGlobal((int)(nPaths * 72));
    IntPtr ma = Marshal.AllocHGlobal((int)(nModes * 64));
    try {
      uint p = nPaths, m = nModes;
      if (QueryDisplayConfigRaw(2, ref p, pa, ref m, ma, IntPtr.Zero) != 0) return "";
      var seen = new System.Collections.Generic.HashSet<string>();
      for (int i = 0; i < p; i++) {
        int off = i * 72;
        var src = new SGDC_SRC_NAME();
        src.header.type = 1; src.header.size = (uint)Marshal.SizeOf(typeof(SGDC_SRC_NAME));
        src.header.adapterId.LowPart = (uint)Marshal.ReadInt32(pa, off);
        src.header.adapterId.HighPart = Marshal.ReadInt32(pa, off + 4);
        src.header.id = (uint)Marshal.ReadInt32(pa, off + 8);
        if (DisplayConfigGetDeviceInfo(ref src) != 0) continue;
        string dev = src.viewGdiDeviceName;
        if (string.IsNullOrEmpty(dev) || !seen.Add(dev.ToUpperInvariant())) continue;

        var aci = new SGDC_ACI();
        aci.header.type = 9; aci.header.size = (uint)Marshal.SizeOf(typeof(SGDC_ACI));
        aci.header.adapterLow = (uint)Marshal.ReadInt32(pa, off + 20);
        aci.header.adapterHigh = Marshal.ReadInt32(pa, off + 24);
        aci.header.id = (uint)Marshal.ReadInt32(pa, off + 28);
        if (DisplayConfigGetDeviceInfo(ref aci) != 0) continue;
        if ((aci.value & 1) == 0) {                   // 不支持 HDR 的屏跳过（不乱设）
          sb.Append("R|").Append(dev).Append("|SKIP(该屏不支持HDR)").Append((char)10);
          continue;
        }
        var acs = new SGDC_ACS();
        acs.header.type = 10;                         // SET_ADVANCED_COLOR_STATE
        acs.header.size = (uint)Marshal.SizeOf(typeof(SGDC_ACS));
        acs.header.adapterLow = (uint)Marshal.ReadInt32(pa, off + 20);
        acs.header.adapterHigh = Marshal.ReadInt32(pa, off + 24);
        acs.header.id = (uint)Marshal.ReadInt32(pa, off + 28);
        acs.enable = (uint)(on != 0 ? 1 : 0);
        int r = DisplayConfigSetDeviceInfo(ref acs);
        // 关键：SET_ADVANCED_COLOR_STATE 只是把新状态写进数据库，必须再 Apply 一次
        // （SetDisplayConfig(QDC_DATABASE_CURRENT)）系统才真正切 HDR ——
        // 少了这一步会「API 返回 0 成功、读回来还是老状态」（实测踩过）。
        int ap = SetDisplayConfigApply(0, IntPtr.Zero, 0, IntPtr.Zero, 4);
        sb.Append("R|").Append(dev).Append("|").Append(r == 0 ? "OK" : ("ERR:set " + r))
          .Append("|apply=").Append(ap == 0 ? "OK" : ("ERR:" + ap)).Append((char)10);
      }
      return sb.ToString();
    } finally { Marshal.FreeHGlobal(pa); Marshal.FreeHGlobal(ma); }
  }

  // ===== 双屏铺满（任意前台窗口，不限于播放器）=====
  /// 从极简 JSON 里取一个整数键（避免引入 JSON 依赖）
  static long JsInt(string js, string key) {
    try {
      int k = js.IndexOf("\"" + key + "\"");
      if (k < 0) return 0;
      int c = js.IndexOf(":", k);
      if (c < 0) return 0;
      int s = c + 1;
      while (s < js.Length && (js[s] == 32 || js[s] == 9)) s++;
      int e = s;
      while (e < js.Length && (char.IsDigit(js[e]) || js[e] == 45)) e++;
      long v;
      return long.TryParse(js.Substring(s, e - s), out v) ? v : 0;
    } catch { return 0; }
  }

  /// 只设置「指定某一台屏」的 HDR（用于基线还原：各屏原本状态可能不同）
  /// 返回 OK / ERR:... 。与 HDRSetAll 同源：SET 之后必须再 Apply 一次才真正生效。
  public static string HDRSetDev(string dev, int on) {
    if (string.IsNullOrEmpty(dev)) return "ERR:设备名为空";
    uint nPaths, nModes;
    if (GetDisplayConfigBufferSizes(2, out nPaths, out nModes) != 0 || nPaths == 0) return "ERR:读取显示配置失败";
    IntPtr pa = Marshal.AllocHGlobal((int)(nPaths * 72));
    IntPtr ma = Marshal.AllocHGlobal((int)(nModes * 64));
    try {
      uint p = nPaths, m = nModes;
      if (QueryDisplayConfigRaw(2, ref p, pa, ref m, ma, IntPtr.Zero) != 0) return "ERR:QueryDisplayConfig 失败";
      for (int i = 0; i < p; i++) {
        int off = i * 72;
        var src = new SGDC_SRC_NAME();
        src.header.type = 1; src.header.size = (uint)Marshal.SizeOf(typeof(SGDC_SRC_NAME));
        src.header.adapterId.LowPart = (uint)Marshal.ReadInt32(pa, off);
        src.header.adapterId.HighPart = Marshal.ReadInt32(pa, off + 4);
        src.header.id = (uint)Marshal.ReadInt32(pa, off + 8);
        if (DisplayConfigGetDeviceInfo(ref src) != 0) continue;
        if (!string.Equals(src.viewGdiDeviceName, dev, StringComparison.OrdinalIgnoreCase)) continue;
        var aci = new SGDC_ACI();
        aci.header.type = 9; aci.header.size = (uint)Marshal.SizeOf(typeof(SGDC_ACI));
        aci.header.adapterLow = (uint)Marshal.ReadInt32(pa, off + 20);
        aci.header.adapterHigh = Marshal.ReadInt32(pa, off + 24);
        aci.header.id = (uint)Marshal.ReadInt32(pa, off + 28);
        if (DisplayConfigGetDeviceInfo(ref aci) != 0) return "ERR:读取 HDR 状态失败";
        if ((aci.value & 1) == 0) return "SKIP:该屏不支持 HDR";
        if (((aci.value >> 1) & 1) == (uint)(on != 0 ? 1 : 0)) return "OK:已是目标状态";
        var acs = new SGDC_ACS();
        acs.header.type = 10; acs.header.size = (uint)Marshal.SizeOf(typeof(SGDC_ACS));
        acs.header.adapterLow = (uint)Marshal.ReadInt32(pa, off + 20);
        acs.header.adapterHigh = Marshal.ReadInt32(pa, off + 24);
        acs.header.id = (uint)Marshal.ReadInt32(pa, off + 28);
        acs.enable = (uint)(on != 0 ? 1 : 0);
        int r = DisplayConfigSetDeviceInfo(ref acs);
        int ap = SetDisplayConfigApply(0, IntPtr.Zero, 0, IntPtr.Zero, 4);
        if (r != 0) return "ERR:set " + r;
        if (ap != 0) return "ERR:apply " + ap;
        return "OK";
      }
      return "ERR:没找到该显示设备";
    } finally { Marshal.FreeHGlobal(pa); Marshal.FreeHGlobal(ma); }
  }

  /// 把某个 ICC 配置文件关联到指定显示设备（基线还原用），内部回读校验
  public static string IccSetPath(string dev, string profile) {
    if (string.IsNullOrEmpty(dev) || string.IsNullOrEmpty(profile)) return "ERR:参数为空";
    if (!System.IO.File.Exists(profile)) return "ERR:配置文件不存在";
    IntPtr hdc = CreateDCW(null, dev, null, IntPtr.Zero);
    if (hdc == IntPtr.Zero) return "ERR:CreateDC 失败";
    try {
      SetICMProfileW(hdc, profile);   // 失败也会返回 TRUE，所以下面必须回读
    } finally { DeleteDC(hdc); }
    var sb = new StringBuilder(600);
    IntPtr h2 = CreateDCW(null, dev, null, IntPtr.Zero);
    if (h2 == IntPtr.Zero) return "ERR:回读 CreateDC 失败";
    try {
      uint sz = 600;
      if (GetICMProfileW(h2, ref sz, sb)) {
        string got = sb.ToString();
        return string.Equals(got, profile, StringComparison.OrdinalIgnoreCase) ? "OK" : ("ERR:回读不符（可能需要管理员权限）");
      }
      return "ERR:回读失败";
    } finally { DeleteDC(h2); }
  }

  /// 把当前前台窗口铺满所有屏幕的包围盒（抖音/浏览器全屏视频、播放器都适用）。
  /// 记录窗口句柄 + 原始位置/样式到 stateFile，供 RestoreForeground 还原。
  /// 返回 OK|<窗口标题> 或 ERR:<原因>
  public static string SpanForeground(string stateFile, int myPid, int mode) {
    IntPtr h = GetForegroundWindow();
    if (h == IntPtr.Zero) return "ERR:没有前台窗口";
    if (h == GetShellWindow() || h == GetDesktopWindow()) return "ERR:当前前台是桌面/任务栏，请先切换到要铺满的窗口";
    var ti = new StringBuilder(300);
    GetWindowTextW(h, ti, 300);
    string title = ti.ToString();
    // 安全闸门：绝不铺满本程序自己的窗口 —— 一旦把自己铺满、而窗口底部被裁掉，
    // 用户就找不到「还原」入口了（实测踩过）。此时改提示用「选择窗口」。
    uint fgPid;
    GetWindowThreadProcessId(h, out fgPid);
    if ((int)fgPid == myPid) {
      return "ERR:当前前台是本程序自己的窗口，不能铺满它（否则你可能找不到「还原」）。请先在右下角「选择窗口」里挑一个别的窗口";
    }
    // 最大化/最小化先还原，否则 SetWindowPos 改不动尺寸
    if (IsIconic(h) || IsZoomed(h)) ShowWindow(h, 9);  // SW_RESTORE
    System.Threading.Thread.Sleep(260);
    SGRECT wr;
    if (!GetWindowRect(h, out wr)) return "ERR:读取窗口位置失败";
    if (wr.Right - wr.Left <= 0 || wr.Bottom - wr.Top <= 0) return "ERR:窗口尺寸无效（可能已被最小化）";
    return SpanHandle(h, stateFile, myPid, mode);
  }

  /// 还原上一次被铺满的窗口（按记录里的句柄找，不依赖当前前台）
  public static string RestoreForeground(string stateFile) {
    string js;
    try { js = System.IO.File.ReadAllText(stateFile); } catch { return "ERR:没有可恢复的记录（先点一次铺满）"; }
    IntPtr h = new IntPtr(JsInt(js, "hwnd"));
    if (h == IntPtr.Zero || !IsWindow(h)) return "ERR:原窗口已关闭";
    int x = (int)JsInt(js, "x"), y = (int)JsInt(js, "y");
    int w = (int)JsInt(js, "w"), hh = (int)JsInt(js, "h");
    long st = JsInt(js, "style");
    if (st != 0) SetWindowLongPtr(h, GWL_STYLE, new IntPtr(st));
    SetWindowPos(h, IntPtr.Zero, x, y, w, hh, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW);
    return "OK";
  }

  static string EscW(string s) {
    if (s == null) return "";
    return s.Replace("|", "/").Replace((char)13, (char)32).Replace((char)10, (char)32);
  }

  /// 列出「可以被铺满的窗口」：可见、有标题、尺寸正常、非本程序、非桌面/任务栏。
  /// 每行 W|hwnd|宽x高|标题。给用户一个可选项，避免只能对"当前前台"下手。
  /// myPid 必须由 Rust 侧传入（应用自己的 PID）—— 因为这段 C# 跑在 powershell.exe 里，
  /// GetCurrentProcessId() 拿到的是 PowerShell 的 PID，拿它排除「自己」是错的（实测踩过）。
  public static string ListWindows(int myPid) {
    var sb = new StringBuilder();
    IntPtr shell = GetShellWindow();
    IntPtr desk = GetDesktopWindow();
    try {
      EnumWindows(delegate(IntPtr h, IntPtr p) {
        if (!IsWindowVisible(h)) return true;
        if (h == shell || h == desk) return true;
        var t = new StringBuilder(300);
        GetWindowTextW(h, t, 300);
        string title = t.ToString();
        if (title.Length == 0) return true;
        uint pid;
        GetWindowThreadProcessId(h, out pid);
        if ((int)pid == myPid) return true;        // 不列自己的窗口（铺满自己会找不到还原入口）
        long ex = GetWindowLongPtr(h, -20).ToInt64();   // GWL_EXSTYLE
        if ((ex & 0x00000080L) != 0) return true;  // WS_EX_TOOLWINDOW：工具窗/浮动提示，跳过
        SGRECT r;
        if (!GetWindowRect(h, out r)) return true;
        int w = r.Right - r.Left, hh = r.Bottom - r.Top;
        if (w < 120 || hh < 90) return true;       // 过滤托盘小窗、0 尺寸窗
        sb.Append("W|").Append(h.ToInt64()).Append("|").Append(w).Append("x").Append(hh)
          .Append("|").Append(EscW(title)).Append((char)10);
        return true;
      }, IntPtr.Zero);
    } catch { }
    return sb.ToString();
  }

  /// 真正干活的实现：对**指定句柄**动手（不依赖前台窗口）。
  /// 必须拆出来：SetForegroundWindow 常常被 Windows 拒绝（后台程序不能抢前台），
  /// 于是原来「先置前再走前台那套逻辑」会把**别人**的窗口铺满 —— 实测拿记事本当靶子，
  /// 结果铺满的是当时真正在前台的抖音窗口。
  static string SpanHandle(IntPtr h, string stateFile, int myPid, int mode) {
    StringBuilder tb = new StringBuilder(512);
    GetWindowTextW(h, tb, 300);
    string title = tb.ToString();
    // 这两句原来在读前台窗口的那段里，搬到 SpanHandle 时必须一起带上（否则 wr 未声明）
    SGRECT wr;
    if (!GetWindowRect(h, out wr)) return "ERR:读取窗口位置失败";
    if (wr.Right - wr.Left <= 0 || wr.Bottom - wr.Top <= 0) return "ERR:窗口尺寸无效（可能已被最小化）";
    long oldStyle = GetWindowLongPtr(h, GWL_STYLE).ToInt64();
    int vx0 = GetSystemMetrics(76), vy0 = GetSystemMetrics(77);
    int vw0 = GetSystemMetrics(78), vh0 = GetSystemMetrics(79);
    // 防呆：已经处于铺满状态、且备份记录属于同一个窗口时，不要再覆盖备份
    // （否则连点两次「铺满」会把备份写成铺满后的矩形，「还原」就再也回不去了）
    bool alreadySpanned = Math.Abs(wr.Left - vx0) < 4 && Math.Abs(wr.Top - vy0) < 4
      && Math.Abs((wr.Right - wr.Left) - vw0) < 8 && Math.Abs((wr.Bottom - wr.Top) - vh0) < 8;
    // 以主屏为基准时，铺满后的矩形不是包围盒，而是「包围盒宽 × 主屏高」，要按这个口径判断
    if (mode == 1) {
      int ph0 = GetSystemMetrics(1); // SM_CYSCREEN = 主屏高
      alreadySpanned = Math.Abs(wr.Left - vx0) < 4 && Math.Abs(wr.Top) < 4
        && Math.Abs((wr.Right - wr.Left) - vw0) < 8 && Math.Abs((wr.Bottom - wr.Top) - ph0) < 8;
    }
    bool sameWin = false;
    try {
      if (System.IO.File.Exists(stateFile)) {
        sameWin = JsInt(System.IO.File.ReadAllText(stateFile), "hwnd") == h.ToInt64();
      }
    } catch { }
    if (!(alreadySpanned && sameWin)) {
      string json = "{\"hwnd\":" + h.ToInt64() + ",\"x\":" + wr.Left + ",\"y\":" + wr.Top
        + ",\"w\":" + (wr.Right - wr.Left) + ",\"h\":" + (wr.Bottom - wr.Top)
        + ",\"style\":" + oldStyle + "}";
      try { System.IO.File.WriteAllText(stateFile, json); } catch { }
    }
    long s = oldStyle;
    s &= ~((long)WS_CAPTION | (long)WS_THICKFRAME);
    s |= (long)WS_POPUP;
    SetWindowLongPtr(h, GWL_STYLE, new IntPtr(s));
    int vx = GetSystemMetrics(76), vy = GetSystemMetrics(77);
    int vw = GetSystemMetrics(78), vh = GetSystemMetrics(79);
    if (mode == 1) {
      // 「以主屏为基准」：宽度照样横跨整块桌面，但高度只取**主屏高度**并对齐主屏顶端。
      // 为什么需要：两块屏分辨率不同（本机主屏 3840x2160 横、副屏 2400x3840 竖），
      // 把窗口放大到整个包围盒(4160x2560)再整体缩放，较短的那块屏底部必然被裁掉；
      // 用主屏高度 → 主屏画面完整无裁切，竖屏那块上下留边而不是切掉画面（用户实测反馈）。
      int ph = GetSystemMetrics(1); // SM_CYSCREEN
      if (ph > 0 && ph < vh) {
        vh = ph;
        vy = 0;
      }
    }
    SetWindowPos(h, IntPtr.Zero, vx, vy, vw, vh, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW);
    return "OK|" + (string.IsNullOrEmpty(title) ? "（无标题窗口）" : title);
  }

  /// 铺满指定的窗口（按 hwnd）。直接对目标句柄动手，不依赖前台窗口。
  public static string SpanWindow(long hwnd, string stateFile, int myPid, int mode) {
    IntPtr h = new IntPtr(hwnd);
    if (!IsWindow(h)) return "ERR:该窗口已关闭";
    if (IsIconic(h)) ShowWindow(h, 9);
    // 直接对目标句柄动手，**不**依赖 SetForegroundWindow
    // （它会失败，导致铺满的是当时真正在前台的**别人**的窗口 —— 实测踩过）
    return SpanHandle(h, stateFile, myPid, mode);
  }

  /// 对指定显示设备执行 DDC/CI（VCP 读或写），返回是否命中；retries 为最大重试次数
  /// 加固（v0.2.8）：① 整体 try/catch——显卡驱动崩溃/TDR 期间 dxva2 调用可能抛异常或返回无效句柄，
  ///   此前无保护会让异常沿 EnumDisplayMonitors 回调冒泡、甚至崩掉承载的 PowerShell 进程；
  ///   ② DestroyPhysicalMonitors 放进 finally——确保任何路径都释放物理显示器句柄，防泄漏；
  ///   ③ 单次失败不再盲目 sleep 重试——驱动复位循环里 DDC 反复超时是加剧 TDR 的元凶，快速失败更安全。
  static bool TryVCP(string dev, byte code, bool write, uint val, int retries, out uint cur) {
    cur = 0;
    var curBox = new uint[1];
    var anyBox = new bool[1];
    try {
      EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, (h, hdc, rc, lp) => {
        var mi = new SGMONITORINFO();
        mi.cb = Marshal.SizeOf(typeof(SGMONITORINFO));
        if (!GetMonitorInfo(h, ref mi)) return true;
        if (!mi.szDevice.Equals(dev, StringComparison.OrdinalIgnoreCase)) return true;
        uint n;
        if (!GetNumberOfPhysicalMonitorsFromHMONITOR(h, out n) || n == 0) return true;
        var arr = new SGPHYS_MON[n];
        if (!GetPhysicalMonitorsFromHMONITOR(h, n, arr)) return true;
        try {
          foreach (var pm in arr) {
            if (pm.h == IntPtr.Zero) continue;
            if (write) {
              // DDC 写入偶发失败，重试；一旦成功即止
              for (int t = 0; t < retries && !anyBox[0]; t++) {
                if (SetVCPFeature(pm.h, code, val)) anyBox[0] = true;
                else if (t + 1 < retries) System.Threading.Thread.Sleep(80);
              }
            } else {
              // 显示器 DDC 响应慢：单次读取常失败，必须重试
              // 注意：变量不能叫 cur/max——与外层参数 out cur 同名的局部变量会 C# 编译失败
              uint curVal = 0, maxVal = 0;
              bool ok = false;
              for (int t = 0; t < retries && !ok; t++) {
                ok = GetVCPFeatureAndVCPFeatureReply(pm.h, code, IntPtr.Zero, ref curVal, ref maxVal);
                if (!ok && t + 1 < retries) System.Threading.Thread.Sleep(80);
              }
              if (ok) { curBox[0] = curVal; anyBox[0] = true; }
            }
          }
        } finally {
          // 无论读写成败都释放句柄，避免物理显示器句柄泄漏（泄漏会在热插拔/驱动复位后累积）
          try { DestroyPhysicalMonitors(n, arr); } catch { }
        }
        return true;
      }, IntPtr.Zero);
    } catch {
      // 驱动栈异常（如 nvlddmkm TDR 崩溃循环、显示器刚被热插拔）时静默快速失败，
      // 不再让异常向上冒泡。返回 false 让上层按“DDC 不可用”走 WMI/提示兜底。
      return false;
    }
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

  // ---------- 屏幕电源控制（方案A：系统级 + DDC 精细） ----------
  [DllImport("user32.dll", SetLastError = true)]
  static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam, uint fuFlags, uint uTimeout, out IntPtr lpdwResult);
  [DllImport("user32.dll")] static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);

  /// 系统级关闭所有显示器：一次让全部屏进入待机（含 DDC 不通的屏，如本机小米）。
  /// 必须用 SendMessageTimeout 而非 SendMessage：广播是同步的，遇到无响应窗口会永久卡死
  /// （实测 SendMessage 广播卡住 90 秒以上），SMTO_ABORTIFHUNG + 超时保证必然返回。
  public static string ScreenOff() {
    IntPtr r;
    SendMessageTimeout((IntPtr)0xFFFF, 0x0112, (IntPtr)0xF170, (IntPtr)2, 0x0002, 3000, out r);
    return "OK";
  }

  /// 唤醒屏幕：合成一次 Shift 按下/抬起（无副作用的真实输入事件，可解除显示器待机）
  public static string ScreenWake() {
    keybd_event(0x10, 0, 0, UIntPtr.Zero);
    keybd_event(0x10, 0, 2, UIntPtr.Zero);
    return "OK";
  }

  /// 读显示器电源模式（VCP 0xD6）：1=开 2=待机 4=软关 5=硬关
  public static string DDCPowerRead(string dev) {
    uint cur;
    return TryVCP(dev, 0xD6, false, 0, 5, out cur) ? cur.ToString() : "ERR";
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

  /// 设置指定显示设备的分辨率/刷新率（就地改 DEVMODE，只写宽/高/频率 + dmFields，其余字节原样保留）。
  /// 返回 OK / ERR:<原因>。w/h 为 0 表示该项不修改；hz 为 0 表示不修改刷新率。
  /// 用于驱动崩溃后把降级的分辨率恢复回显示器原生模式（等价于系统「设置→显示」里改分辨率）。
  public static string SetMode(string dev, uint w, uint h, uint hz) {
    var b = AllocDevMode();
    try {
      if (!EnumDisplaySettingsW(dev, -1, b)) return "ERR:读取当前显示模式失败";
      uint fields = 0;
      if (w != 0 && h != 0) {
        Marshal.WriteInt32(b, OFF_PELSWIDTH, (int)w);
        Marshal.WriteInt32(b, OFF_PELSHEIGHT, (int)h);
        fields |= DM_PELSWIDTH | DM_PELSHEIGHT;
      }
      if (hz != 0) {
        Marshal.WriteInt32(b, OFF_FREQUENCY, (int)hz);
        fields |= DM_DISPLAYFREQUENCY;
      }
      if (fields == 0) return "ERR:未指定要修改的项";
      Marshal.WriteInt32(b, OFF_DMFIELDS, (int)fields);
      Marshal.WriteInt16(b, OFF_DMSIZE, (short)DEVMODE_SIZE);
      int r = ChangeDisplaySettingsExW(dev, b, IntPtr.Zero, CDS_UPDATEREGISTRY | CDS_GLOBAL, IntPtr.Zero);
      return r == 0 ? "OK" : ("ERR:ChangeDisplaySettingsEx 返回 " + r);
    } finally { Marshal.FreeHGlobal(b); }
  }

  // ===== 显示器健康：KVM 切换 / EDID 重新协商异常的事件读取与修复 =====

  /// 读取最近的显示 / 设备链路异常事件（KVM 切换、显示器突然移除、驱动看门狗超时）。
  /// 返回每行: E|时间|等级|来源|事件ID|摘要
  /// 数据源：Kernel-PnP 设备管理（surprise removed = 显示器从系统里消失）、
  ///         Kernel-PnP 驱动看门狗（长时间阻塞）、System（驱动重置 TDR 4101）。
  public static string ReadDisplayEvents(int minutes) {
    var sb = new System.Text.StringBuilder();
    string[] logs = new string[] {
      "Microsoft-Windows-Kernel-PnP/Device Management",
      "Microsoft-Windows-Kernel-PnP/Driver Watchdog",
      "System"
    };
    long windowMs = (long)minutes * 60000L;
    foreach (string ln in logs) {
      try {
        var q = new System.Diagnostics.Eventing.Reader.EventLogQuery(
          ln, System.Diagnostics.Eventing.Reader.PathType.LogName,
          "*[System[TimeCreated[timediff(@SystemTime) <= " + windowMs + "]]]");
        var rd = new System.Diagnostics.Eventing.Reader.EventLogReader(q);
        int n = 0;
        for (var e = rd.ReadEvent(); e != null && n < 40; e = rd.ReadEvent()) {
          string prov = e.ProviderName == null ? "" : e.ProviderName;
          string msg = "";
          try { msg = e.FormatDescription(); } catch { }
          if (msg == null) msg = "";
          msg = msg.Replace((char)13, (char)32).Replace((char)10, (char)32);
          if (msg.Length > 120) msg = msg.Substring(0, 120);
          string low = msg.ToLower();
          bool keep = false;
          if (low.IndexOf("display") >= 0 || low.IndexOf("monitor") >= 0) keep = true;
          if (prov.IndexOf("PnP") >= 0) keep = true;
          if (e.Id == 4101 || e.Id == 1010 || e.Id == 900 || e.Id == 902 || e.Id == 933 || e.Id == 901) keep = true;
          if (!keep) continue;
          int lv = e.Level == null ? 4 : (int)e.Level;
          string lvl = lv == 1 ? "严重" : (lv == 2 ? "错误" : (lv == 3 ? "警告" : "信息"));
          string ts = e.TimeCreated == null ? "" : ((DateTime)e.TimeCreated).ToString("MM-dd HH:mm:ss");
          sb.Append("E|").Append(ts).Append("|").Append(lvl).Append("|").Append(prov)
            .Append("|").Append(e.Id).Append("|").Append(msg).Append((char)10);
          n++;
        }
        rd.Dispose();
      } catch { }
    }
    return sb.ToString();
  }

  /// 强制显示链路重新协商：先把刷新率降下来再切回原模式（两次 ChangeDisplaySettingsEx）。
  /// 用于修复 KVM 切换 / EDID 重新协商后常见的「画面发白发亮（色彩格式被降级成 YCbCr）、
  /// 清晰度变差、抖动花屏、刷新率异常」。返回 OK:<模式> 或 ERR:<原因>。
  public static string ForceReNegotiate(string dev) {
    var orig = AllocDevMode();
    try {
      if (!EnumDisplaySettingsW(dev, -1, orig)) return "ERR:读取当前显示模式失败";
      int w = Marshal.ReadInt32(orig, OFF_PELSWIDTH);
      int h = Marshal.ReadInt32(orig, OFF_PELSHEIGHT);
      int hz = Marshal.ReadInt32(orig, OFF_FREQUENCY);
      if (w <= 0 || h <= 0) return "ERR:当前模式无效";
      if (hz != 60) {
        var tmp = AllocDevMode();
        try {
          if (EnumDisplaySettingsW(dev, -1, tmp)) {
            Marshal.WriteInt32(tmp, OFF_FREQUENCY, 60);
            Marshal.WriteInt32(tmp, OFF_DMFIELDS, (int)DM_DISPLAYFREQUENCY);
            Marshal.WriteInt16(tmp, OFF_DMSIZE, (short)DEVMODE_SIZE);
            ChangeDisplaySettingsExW(dev, tmp, IntPtr.Zero, CDS_UPDATEREGISTRY | CDS_GLOBAL, IntPtr.Zero);
            System.Threading.Thread.Sleep(1200);
          }
        } finally { Marshal.FreeHGlobal(tmp); }
      }
      Marshal.WriteInt32(orig, OFF_DMFIELDS, (int)(DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_DISPLAYORIENTATION));
      Marshal.WriteInt16(orig, OFF_DMSIZE, (short)DEVMODE_SIZE);
      int r = ChangeDisplaySettingsExW(dev, orig, IntPtr.Zero, CDS_UPDATEREGISTRY | CDS_GLOBAL, IntPtr.Zero);
      return r == 0 ? ("OK:" + w + "x" + h + "@" + hz) : ("ERR:ChangeDisplaySettingsEx 返回 " + r);
    } finally { Marshal.FreeHGlobal(orig); }
  }

  /// 每台在用的屏的当前模式：M|设备名|宽|高|刷新率|方向
  public static string ModeDetail() {
    var sb = new System.Text.StringBuilder();
    for (int i = 1; i <= 12; i++) {
      string dev = "\\\\.\\DISPLAY" + i;
      var dm = AllocDevMode();
      try {
        if (EnumDisplaySettingsW(dev, -1, dm)) {
          int w = Marshal.ReadInt32(dm, OFF_PELSWIDTH);
          int h = Marshal.ReadInt32(dm, OFF_PELSHEIGHT);
          int hz = Marshal.ReadInt32(dm, OFF_FREQUENCY);
          int ori = Marshal.ReadInt32(dm, OFF_ORIENTATION);
          if (w > 0 && h > 0)
            sb.Append("M|").Append(dev).Append("|").Append(w).Append("|").Append(h).Append("|").Append(hz).Append("|").Append(ori).Append((char)10);
        }
      } finally { Marshal.FreeHGlobal(dm); }
    }
    return sb.ToString();
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

  /// 列出所有「活动的输出端点」，每行：A|端点id|名称|音量|静音|可调|形态
  /// 为什么需要：部分 HDMI/DP 显示器音频端点是**固定音量**，Windows 自己的音量条
  /// 也调不动。此时不该让滑块假装能用，而应让用户从列表里挑一个真正能调的设备。
  struct SGWA { public int l; public int t; public int r; public int b; }
  [DllImport("user32.dll", CharSet = CharSet.Auto)]
  static extern bool SystemParametersInfo(uint act, uint u, ref SGWA p, uint f);

  /// 主屏「工作区」右下角（已排除任务栏）——通知气泡要贴这个角
  /// 用 SPI_GETWORKAREA(0x30) 而不是自己算：它带正确的原点偏移且自动避开任务栏
  public static string PrimaryWorkArea() {
    try {
      SGWA w = new SGWA();
      if (SystemParametersInfo(0x0030, 0, ref w, 0) && w.r > w.l && w.b > w.t) {
        return "WA|" + w.l + "|" + w.t + "|" + w.r + "|" + w.b;
      }
    } catch { }
    return "WA|0|0|0|0";
  }

  public static string AudioList() {
    var sb = new StringBuilder();
    try {
      var en = (SGIMMEnum)(object)new SGMMEnumerator();
      SGIMMColl coll;
      // mask = 0x0F(DEVICE_STATEMASK_ALL)：把「未插入/未激活/已禁用」的端点也列出来，
      // 否则用户会问「我的副屏音频设备去哪了」——副屏的 HDMI/DP 音频常常处于未激活态。
      if (en.EnumAudioEndpoints(0, 15, out coll) != 0 || coll == null) return "ERR:枚举输出端点失败";
      int cnt;
      if (coll.GetCount(out cnt) != 0) return "ERR:读取端点数量失败";
      for (int i = 0; i < cnt; i++) {
        // 注意：这次特意连「未激活/未插入」的端点一起枚举（用户要看到副屏设备），
        // 而对这类端点做 Activate 会抛 0x80070003「系统找不到指定的路径」。
        // 所以逐个用 try 包住：某个端点坏掉只是跳过它，不能让整个列表打不开。
        try {
        SGIMMDev dev;
        if (coll.Item(i, out dev) != 0 || dev == null) continue;
        string id = "";
        try { dev.GetId(out id); } catch { }
        string name = "";
        string ff = "0";
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
        } catch { }
        SGEndVol vol = VolOf(dev);
        if (vol == null) continue;
        float lvl = 0; uint mask = 0; int mute = 0;
        try { vol.GetMasterVolumeLevelScalar(out lvl); } catch { }
        try { vol.QueryHardwareSupport(out mask); } catch { }
        try { vol.GetMute(out mute); } catch { }
        sb.Append("A|").Append(Esc(id)).Append("|").Append(Esc(name)).Append("|")
          .Append(Math.Round(lvl * 100)).Append("|").Append(mute != 0 ? 1 : 0).Append("|")
          .Append((mask & 1) != 0 ? 1 : 0).Append("|").Append(ff).Append("|")
          .Append(IsDefaultId(id) ? 1 : 0).Append((char)10);
        } catch { }
      }
      return sb.ToString();
    } catch (Exception e) { return "ERR:" + Esc(e.Message); }
  }

  /// 该端点是否就是 Windows 当前的默认输出设备
  static bool IsDefaultId(string id) {
    try {
      SGIMMDev d = DefaultDev();
      if (d == null) return false;
      string did = "";
      d.GetId(out did);
      return did == id;
    } catch { return false; }
  }

  static SGIMMDev DevById(string id) {
    if (id == null || id.Length == 0) return DefaultDev();
    try {
      var en = (SGIMMEnum)(object)new SGMMEnumerator();
      SGIMMColl coll;
      if (en.EnumAudioEndpoints(0, 1, out coll) != 0 || coll == null) return DefaultDev();
      int cnt; coll.GetCount(out cnt);
      for (int i = 0; i < cnt; i++) {
        SGIMMDev d;
        if (coll.Item(i, out d) != 0 || d == null) continue;
        string did = "";
        try { d.GetId(out did); } catch { }
        if (did == id) return d;
      }
    } catch { }
    return DefaultDev();
  }

  /// 设定指定输出端点的音量（id 为空则用系统默认端点）
  public static string AudioSetDev(string id, uint v) {
    try {
      SGIMMDev dev = DevById(id);
      if (dev == null) return "ERR:没有可用的输出设备";
      SGEndVol vol = VolOf(dev);
      if (vol == null) return "ERR:端点激活失败";
      if (v > 100) v = 100;
      int r = vol.SetMasterVolumeLevelScalar(v / 100f, IntPtr.Zero);
      return r == 0 ? "OK" : ("ERR:设置失败 hr=" + r + "（该端点是固定音量，换一个输出设备）");
    } catch (Exception e) { return "ERR:" + Esc(e.Message); }
  }

  /// 指定输出端点静音开关
  public static string AudioMuteDev(string id, uint on) {
    try {
      SGIMMDev dev = DevById(id);
      if (dev == null) return "ERR:没有可用的输出设备";
      SGEndVol vol = VolOf(dev);
      if (vol == null) return "ERR:端点激活失败";
      return vol.SetMute(on != 0 ? 1 : 0, IntPtr.Zero) == 0 ? "OK" : "ERR:静音设置失败";
    } catch (Exception e) { return "ERR:" + Esc(e.Message); }
  }
}

// ============ 电池设备（SetupAPI + Battery IOCTL） ============
// 所有带电池的连接设备——笔记本内电池 / USB·2.4G 无线 HID 键鼠 / 蓝牙（含 BLE GATT
// 电量服务 0x180F）——都在 Battery 设备类注册接口；电池驱动注册的设备接口 GUID 恰好
// 就是 GUID_DEVCLASS_BATTERY(72631E54-78A4-11D0-BCF7-00AA00B7B32A)，因此用
// DIGCF_DEVICEINTERFACE | DIGCF_PRESENT 枚举。与驱动栈状态无关，不碰注册表只读。
[StructLayout(LayoutKind.Sequential)]
public struct SGIFACE_DATA { public int cb; public Guid cls; public uint Flags; public IntPtr Reserved; }

[StructLayout(LayoutKind.Sequential)]
public struct SGDEVINFO_DATA { public int cb; public Guid cls; public uint DevInst; public IntPtr Reserved; }

public class SGBatt {
  // IOCTL_BATTERY_*：FILE_DEVICE_BATTERY=0x29，FILE_READ_ACCESS=1，METHOD_BUFFERED=0
  // CTL_CODE(0x29, Function, 0, 1) = 0x290000|0x4000|(Function<<2)
  const uint IOCTL_TAG = 0x294040;        // QUERY_TAG      (Function 0x10)
  const uint IOCTL_INFO = 0x294044;       // QUERY_INFORMATION (Function 0x11)
  const uint IOCTL_STATUS = 0x29404c;     // QUERY_STATUS   (Function 0x13)
  // BATTERY_QUERY_INFORMATION.InformationLevel
  const int LEVEL_INFO = 0, LEVEL_NAME = 4;
  // BATTERY_STATUS.PowerState 位（ntddbat.h）
  const uint ST_ON_LINE = 0x1, ST_DISCHARGING = 0x2, ST_CHARGING = 0x4;

  [DllImport("setupapi.dll", CharSet = CharSet.Unicode)] static extern IntPtr SetupDiGetClassDevsW(ref Guid g, string e, IntPtr p, uint f);
  [DllImport("setupapi.dll", CharSet = CharSet.Unicode)] static extern bool SetupDiEnumDeviceInterfaces(IntPtr h, IntPtr di, ref Guid g, uint i, ref SGIFACE_DATA d);
  [DllImport("setupapi.dll", CharSet = CharSet.Unicode)] static extern bool SetupDiGetDeviceInterfaceDetailW(IntPtr h, ref SGIFACE_DATA d, IntPtr det, uint sz, out uint need, ref SGDEVINFO_DATA di);
  [DllImport("setupapi.dll", CharSet = CharSet.Unicode)] static extern bool SetupDiGetDeviceRegistryPropertyW(IntPtr h, ref SGDEVINFO_DATA di, uint prop, out uint t, IntPtr buf, uint sz, out uint need);
  [DllImport("setupapi.dll")] static extern bool SetupDiDestroyDeviceInfoList(IntPtr h);
  [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern IntPtr CreateFileW(string p, uint a, uint s, IntPtr sa, uint disp, uint flags, IntPtr tpl);
  [DllImport("kernel32.dll", SetLastError = true)] static extern bool DeviceIoControl(IntPtr h, uint ctl, IntPtr inp, uint ins, IntPtr outp, uint outs, out uint ret, IntPtr ov);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);

  static string Esc(string s) {
    if (s == null) return "";
    return s.Replace("|", "/").Replace("\r", " ").Replace("\n", " ");
  }

  // 注册表字符串属性（SPDRP_FRIENDLYNAME=0x0C / SPDRP_DEVICEDESC=0）
  static string RegProp(IntPtr hdev, ref SGDEVINFO_DATA di, uint prop) {
    try {
      uint t, need;
      if (!SetupDiGetDeviceRegistryPropertyW(hdev, ref di, prop, out t, IntPtr.Zero, 0, out need)) return "";
      if (need == 0 || need > 1024) return "";
      IntPtr buf = Marshal.AllocHGlobal((int)need);
      try {
        if (SetupDiGetDeviceRegistryPropertyW(hdev, ref di, prop, out t, buf, need, out need)) {
          string s = Marshal.PtrToStringUni(buf);
          if (s != null) return s.Trim();
        }
      } finally { Marshal.FreeHGlobal(buf); }
    } catch (Exception) { }
    return "";
  }

  /// 枚举所有带电池的设备。行: B|<名称>|<百分比或-1>|<状态位1=充电,2=接电>|<连接类型>
  public static string List() {
    var sb = new StringBuilder();
    try {
      Guid bat = new Guid(0x72631E54, 0x78A4, 0x11D0, 0xBC, 0xF7, 0x00, 0xAA, 0x00, 0xB7, 0xB3, 0x2A);
      IntPtr hdev = SetupDiGetClassDevsW(ref bat, null, IntPtr.Zero, 0x12); // DIGCF_PRESENT|DIGCF_DEVICEINTERFACE
      if (hdev == new IntPtr(-1)) return "ERR:SetupDiGetClassDevs 失败";
      try {
        uint idx = 0;
        while (true) {
          var ifd = new SGIFACE_DATA(); ifd.cb = Marshal.SizeOf(typeof(SGIFACE_DATA));
          if (!SetupDiEnumDeviceInterfaces(hdev, IntPtr.Zero, ref bat, idx, ref ifd)) break;
          idx++;
          try { EmitBattery(hdev, ref ifd, ref bat, sb); } catch (Exception) { }
        }
      } finally { SetupDiDestroyDeviceInfoList(hdev); }
      return sb.ToString();
    } catch (Exception e) { return "ERR:" + Esc(e.Message); }
  }

  static void EmitBattery(IntPtr hdev, ref SGIFACE_DATA ifd, ref Guid bat, StringBuilder sb) {
    // 1) 接口路径。cbSize 在 x64 必须写 8（sizeof 含对齐尾填充，API 校验值），
    //    但 DevicePath 本身在偏移 4（紧跟 DWORD cbSize，x64 实测）——按偏移直读绕开 marshaller
    uint need;
    var di = new SGDEVINFO_DATA(); di.cb = Marshal.SizeOf(typeof(SGDEVINFO_DATA));
    SetupDiGetDeviceInterfaceDetailW(hdev, ref ifd, IntPtr.Zero, 0, out need, ref di); // 预期失败，need 已写入
    if (need == 0 || need > 4096) return;
    IntPtr det = Marshal.AllocHGlobal((int)need);
    try {
      Marshal.WriteInt32(det, 8); // SP_DEVICE_INTERFACE_DETAIL_DATA.cbSize（x64 校验值）
      if (!SetupDiGetDeviceInterfaceDetailW(hdev, ref ifd, det, need, out need, ref di)) return;
      string path = Marshal.PtrToStringUni(new IntPtr(det.ToInt64() + 4));
      if (String.IsNullOrEmpty(path)) return;

      // 2) 连接类型（接口路径前缀判读）
      string pl = path.ToUpperInvariant();
      string conn = "其它";
      if (pl.Contains("BTHLE") || pl.Contains("BTHENUM")) conn = "蓝牙";
      else if (pl.Contains("ACPI")) conn = "内置";
      else if (pl.Contains("HID")) conn = "USB/无线";
      else if (pl.Contains("USB")) conn = "USB";

      // 3) 名称：注册表友好名 → 设备描述 → 电池栈设备名 → 兜底
      string name = RegProp(hdev, ref di, 0x0C);
      if (name.Length == 0) name = RegProp(hdev, ref di, 0);
      bool genericName = name.Length == 0;
      // 内置电池通常无友好名，直接用「内置电池」而非序列号；外设保留电池栈设备名兜底
      if (genericName) name = (conn == "内置") ? "内置电池" : "电池设备";

      // 4) 打开设备查状态（GENERIC_READ|GENERIC_WRITE，FILE_SHARE_RW，OPEN_EXISTING）
      IntPtr b = CreateFileW(path, 0xC0000000u, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
      if (b == new IntPtr(-1)) {
        sb.Append("B|").Append(Esc(name)).Append("|-1|0|").Append(conn).Append((char)10);
        return;
      }
      try {
        // 4a) QUERY_TAG：入参是 ULONG 等待超时（0 = 立即），出参 BatteryTag（0 = 电池不存在）
        IntPtr tagBuf = Marshal.AllocHGlobal(4);
        IntPtr waitIn = Marshal.AllocHGlobal(4);
        try {
          Marshal.WriteInt32(waitIn, 0);
          uint got;
          if (!DeviceIoControl(b, IOCTL_TAG, waitIn, 4, tagBuf, 4, out got, IntPtr.Zero) || got != 4) return;
          int tag = Marshal.ReadInt32(tagBuf);
          if (tag == 0) return;

          // 4b) QUERY_INFORMATION(level=0)：BATTERY_INFORMATION 36 字节（含 CycleCount），
          //     FullChargedCapacity 在偏移 16（绝对模式=mWh，相对模式=100）
          uint full = 0, designed = 0;
          IntPtr qi = Marshal.AllocHGlobal(12);   // {tag, level, atRate}
          IntPtr info = Marshal.AllocHGlobal(48);
          try {
            Marshal.WriteInt32(qi, 0, tag);
            Marshal.WriteInt32(qi, 4, LEVEL_INFO);
            Marshal.WriteInt32(qi, 8, 0);
            if (DeviceIoControl(b, IOCTL_INFO, qi, 12, info, 48, out got, IntPtr.Zero) && got >= 20) {
              full = (uint)Marshal.ReadInt32(info, 16); // FullChargedCapacity
              if (got >= 16) designed = (uint)Marshal.ReadInt32(info, 12); // DesignedCapacity（健康度分母）
            }
            // 名称兜底：外设（USB/无线/蓝牙）注册表名是泛型时，读电池栈 BatteryDeviceName
            if (genericName && conn != "内置") {
              Marshal.WriteInt32(qi, 4, LEVEL_NAME);
              IntPtr nbuf = Marshal.AllocHGlobal(512);
              try {
                if (DeviceIoControl(b, IOCTL_INFO, qi, 12, nbuf, 512, out got, IntPtr.Zero) && got >= 4) {
                  string dn = Marshal.PtrToStringUni(nbuf);
                  if (dn != null && dn.Trim().Length > 0) name = dn.Trim();
                }
              } finally { Marshal.FreeHGlobal(nbuf); }
            }
          } finally { Marshal.FreeHGlobal(qi); Marshal.FreeHGlobal(info); }

          // 4c) QUERY_STATUS：BATTERY_WAIT_STATUS 20 字节入，BATTERY_STATUS 16 字节出
          //     {PowerState(0), Capacity(4), Voltage(8), Rate(12)}
          uint powerState = 0, cap = 0;
          IntPtr ws = Marshal.AllocHGlobal(20);
          IntPtr st = Marshal.AllocHGlobal(16);
          try {
            Marshal.WriteInt32(ws, 0, tag); // 其余字段全 0（不等待、不设阈值）
            if (DeviceIoControl(b, IOCTL_STATUS, ws, 20, st, 16, out got, IntPtr.Zero) && got >= 8) {
              powerState = (uint)Marshal.ReadInt32(st, 0);
              cap = (uint)Marshal.ReadInt32(st, 4);
            }
          } finally { Marshal.FreeHGlobal(ws); Marshal.FreeHGlobal(st); }

          // 5) 百分比：绝对模式 cap*100/full；相对模式（多数 HID 外设）full=100 公式同样成立；
          //    full 读不到但 cap<=100 时视为本身就是百分比
          int percent;
          if (full > 0 && cap <= full) percent = (int)Math.Round(cap * 100.0 / full);
          else if (cap > 0 && cap <= 100) percent = (int)cap;
          else percent = -1;
          if (percent > 100) percent = 100;

          uint stateBits = 0;
          if ((powerState & ST_CHARGING) != 0) stateBits |= 1;
          if ((powerState & ST_ON_LINE) != 0) stateBits |= 2;
          // 健康度：满充容量 / 设计容量（仅当两者合理时给出，否则 -1 不给误导数字）
          int health = -1;
          if (designed > 0 && full > 0 && full <= designed + designed / 20) {
            health = (int)Math.Round(full * 100.0 / designed);
            if (health > 100) health = 100;
            if (health < 1) health = -1;
          }
          sb.Append("B|").Append(Esc(name)).Append("|").Append(percent).Append("|")
            .Append(stateBits).Append("|").Append(conn).Append("|").Append(health)
            .Append("|").Append(cap).Append("|").Append(full).Append((char)10);
        } finally { Marshal.FreeHGlobal(tagBuf); Marshal.FreeHGlobal(waitIn); }
      } finally { CloseHandle(b); }
    } finally { Marshal.FreeHGlobal(det); }
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
    // 走编译缓存加载（见 core_asm_ref 注释：不缓存会每次都重编译整份 C#，拖滑块时界面卡死）
    let csp = match core_asm_ref() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let script = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\nAdd-Type -Path '{cs}';\n$lines = [SGCore]::ListDisplays() -split ([char]10);\n$lines | ForEach-Object {{ Write-Output $_ }};\n$devs = @();\nforeach ($l in $lines) {{ $p = $l -split '\\|'; if ($p[0] -eq 'D') {{ $devs += $p[1] }} }};\n[SGCore]::DisplayNameMap();\nGet-CimInstance -Namespace root/wmi -ClassName WmiMonitorID -ErrorAction SilentlyContinue | ForEach-Object {{\n  $nm = (($_.UserFriendlyName | Where-Object {{ [int]$_ -ne 0 }} | ForEach-Object {{ [char][int]$_ }}) -join '');\n  $inst = $_.InstanceName -replace '_\\d+$', '';\n  if ($_.Active -and $nm -and $nm.Trim()) {{ Write-Output ('N|' + $inst + '|' + $nm) }}\n}};\nforeach ($dev in $devs) {{ $pr = [SGCore]::DDCProbe($dev); $b = ''; $v = ''; if ($pr -eq '1') {{ $b = [SGCore]::DDCRead($dev, [byte]0x10); $v = [SGCore]::DDCRead($dev, [byte]0x62) }}; Write-Output ('P|' + $dev + '|' + $pr + '|' + $b + '|' + $v); Write-Output ('C|' + $dev + '|' + [SGCore]::IccGet($dev)) }};\nGet-CimInstance -Namespace root/wmi -ClassName WmiMonitorBrightness -ErrorAction SilentlyContinue | ForEach-Object {{ $sg = ($_.InstanceName -split '\\\\')[1]; if ($sg) {{ Write-Output ('W|' + $sg + '|' + $_.CurrentBrightness) }} }}",
        cs = csp
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
    //  ③ 1↔1 兜底：仅当「只剩一行没名字」且「恰好还剩一个没被 ①② 用掉的名字」时才猜
    let mut matched: Vec<Option<String>> = Vec::with_capacity(rows.len());
    for (dev, uid, _, _, _, _, _) in rows.iter() {
        let mut name = dc_names
            .get(&dev.to_lowercase())
            .cloned()
            .filter(|n| !n.is_empty());
        if name.is_none() {
            let key = short_id(uid);
            if !key.is_empty() {
                name = names.get(&key).cloned().filter(|n| !n.is_empty());
            }
        }
        matched.push(name);
    }
    {
        // 兜底前必须先算清「哪些名字已经被用掉」——① DisplayConfig 与 ② uid 段两条来源都要计入。
        // 只看 ② 的用量会误配：小米已由 DisplayConfig 匹配到自己的名字，副屏没有 WMI 名字时，
        // 兜底逻辑又把同一个名字塞给副屏，于是两台屏显示成同一个名字（实测 v0.2.9 就是这个现象）。
        let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
        for m in matched.iter().flatten() {
            used.insert(m.clone());
        }
        let unmatched: Vec<usize> = matched
            .iter()
            .enumerate()
            .filter(|(_, m)| m.is_none())
            .map(|(i, _)| i)
            .collect();
        let leftover: Vec<&String> = names.values().filter(|n| !used.contains(*n)).collect();
        if unmatched.len() == 1 && leftover.len() == 1 {
            matched[unmatched[0]] = Some(leftover[0].clone());
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

// ---------- 原生 DDC/CI 快路径（毫秒级，不启动 PowerShell） ----------
//
// 为什么要它：走 PowerShell 时一次 DDC 读写 = 启动进程(~0.4s) + 加载程序集 + I2C 通信，
// 实测单次 get_displays 要 1.9 秒。拖亮度滑块时每一格都等这么一下，手感就是一顿一顿的。
// 直接调 dxva2 的 VCP 接口只要十几毫秒。原生失败会自动回退到下面的 PowerShell 路径。
use std::ffi::c_void;

/// DDC 用到的句柄/指针统一别名
type DdcPtr = *mut c_void;

#[repr(C)]
struct DdcRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct DdcMonitorInfoExW {
    cb_size: u32,
    rc_monitor: DdcRect,
    rc_work: DdcRect,
    flags: u32,
    device: [u16; 32],
}

#[repr(C)]
struct DdcPhysicalMonitor {
    handle: DdcPtr,
    description: [u16; 128],
}

type DdcEnumProc = extern "system" fn(DdcPtr, DdcPtr, *mut DdcRect, isize) -> i32;

#[link(name = "user32")]
extern "system" {
    fn EnumDisplayMonitors(hdc: DdcPtr, clip: DdcPtr, cb: DdcEnumProc, data: isize) -> i32;
    fn GetMonitorInfoW(hmonitor: DdcPtr, mi: *mut DdcMonitorInfoExW) -> i32;
}

#[link(name = "dxva2")]
extern "system" {
    fn GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor: DdcPtr, n: *mut u32) -> i32;
    fn GetPhysicalMonitorsFromHMONITOR(
        hmonitor: DdcPtr,
        n: u32,
        arr: *mut DdcPhysicalMonitor,
    ) -> i32;
    fn SetVCPFeature(hmonitor: DdcPtr, code: u8, value: u32) -> i32;
    fn GetVCPFeatureAndVCPFeatureReply(
        hmonitor: DdcPtr,
        code: u8,
        vct: *mut i32,
        cur: *mut u32,
        max: *mut u32,
    ) -> i32;
    fn DestroyPhysicalMonitors(n: u32, arr: *mut DdcPhysicalMonitor) -> i32;
}

struct DdcFind {
    want: String,
    found: DdcPtr,
}

extern "system" fn ddc_find_cb(h: DdcPtr, _hdc: DdcPtr, _rc: *mut DdcRect, data: isize) -> i32 {
    if data == 0 {
        return 0;
    }
    let ctx = unsafe { &mut *(data as *mut DdcFind) };
    let mut mi: DdcMonitorInfoExW = unsafe { std::mem::zeroed() };
    mi.cb_size = std::mem::size_of::<DdcMonitorInfoExW>() as u32;
    if unsafe { GetMonitorInfoW(h, &mut mi) } != 0 {
        let len = mi.device.iter().position(|c| *c == 0).unwrap_or(mi.device.len());
        let dev = String::from_utf16_lossy(&mi.device[..len]);
        if dev.eq_ignore_ascii_case(&ctx.want) {
            ctx.found = h;
            return 0; // 命中，停止枚举
        }
    }
    1 // 继续找
}

/// 在指定显示器的物理句柄上执行一次操作（自动获取并释放句柄）
fn ddc_with<T>(dev: &str, f: impl FnOnce(DdcPtr) -> T) -> Option<T> {
    let mut ctx = DdcFind {
        want: dev.to_string(),
        found: std::ptr::null_mut(),
    };
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            ddc_find_cb,
            &mut ctx as *mut DdcFind as isize,
        )
    };
    if ctx.found.is_null() {
        return None;
    }
    let mut n: u32 = 0;
    if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(ctx.found, &mut n) } == 0 || n == 0 {
        return None;
    }
    let mut arr: Vec<DdcPhysicalMonitor> = Vec::with_capacity(n as usize);
    for _ in 0..n {
        arr.push(unsafe { std::mem::zeroed() });
    }
    if unsafe { GetPhysicalMonitorsFromHMONITOR(ctx.found, n, arr.as_mut_ptr()) } == 0 {
        return None;
    }
    let out = f(arr[0].handle);
    unsafe { DestroyPhysicalMonitors(n, arr.as_mut_ptr()) };
    Some(out)
}

/// 原生写 VCP（true = 成功）
fn ddc_set(dev: &str, code: u8, value: u32) -> bool {
    ddc_with(dev, |h| unsafe { SetVCPFeature(h, code, value) != 0 }).unwrap_or(false)
}

/// 原生读 VCP 当前值
fn ddc_get(dev: &str, code: u8) -> Option<u32> {
    ddc_with(dev, |h| {
        let (mut vct, mut cur, mut max) = (0i32, 0u32, 0u32);
        if unsafe { GetVCPFeatureAndVCPFeatureReply(h, code, &mut vct, &mut cur, &mut max) } != 0 {
            Some(cur)
        } else {
            None
        }
    })
    .flatten()
}

// ---------- DDC：亮度 / 音量（按显示器精确控制） ----------
fn vcp_op(display_id: &str, code: u8, value: Option<u32>) -> Result<(), String> {
    let dev = resolve_dev(display_id)?;
    // 快路径：原生写入。拖亮度/音量滑块走的就是这里，省掉每次约 0.5 秒的 PowerShell 启动开销。
    if let Some(v) = value {
        if ddc_set(&dev, code, v) {
            return Ok(());
        }
    }
    // 慢路径：原 PowerShell 实现（原生不可用时兜底，保证兼容性）
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
// SystemAudio 结构体定义在 platform/mod.rs（三平台共享，非 Windows 由存根实现兜底）

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

// ---------- 电池设备（Battery 类设备：内电池 / USB·无线 HID / 蓝牙） ----------
// BatteryInfo 结构体定义在 platform/mod.rs（三平台共享，非 Windows 由存根实现兜底）

pub fn get_batteries() -> Result<Vec<BatteryInfo>, String> {
    let out = ps_core("[SGBatt]::List()")?;
    let t = out.trim();
    if t.starts_with("ERR:") {
        return Err(t.strip_prefix("ERR:").unwrap_or(t).to_string());
    }
    let mut list = Vec::new();
    for line in out.lines() {
        let t = line.trim_end();
        // B|<名称>|<百分比或-1>|<状态位1=充电,2=接电>|<连接类型>|<健康度>|<当前容量mWh>|<满充容量mWh>
        // 备注：打开设备失败时只输出前 5 段（后三项缺省）
        let rest = match t.strip_prefix("B|") {
            Some(r) => r,
            None => continue,
        };
        let p: Vec<&str> = rest.split('|').collect();
        if p.len() < 4 {
            continue;
        }
        let percent: i32 = p[1].trim().parse().unwrap_or(-1);
        let state: u32 = p[2].trim().parse().unwrap_or(0);
        let health: i32 = if p.len() > 4 { p[4].trim().parse().unwrap_or(-1) } else { -1 };
        let cap_mwh: u32 = if p.len() > 5 { p[5].trim().parse().unwrap_or(0) } else { 0 };
        let full_mwh: u32 = if p.len() > 6 { p[6].trim().parse().unwrap_or(0) } else { 0 };
        list.push(BatteryInfo {
            name: p[0].to_string(),
            percent,
            charging: state & 1 != 0,
            on_ac: state & 2 != 0,
            conn: p[3].to_string(),
            health,
            cap_mwh,
            full_mwh,
        });
    }
    Ok(list)
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

/// 色彩空间别名表：用户目录 / 系统色彩目录里只要放着名字匹配的 .icm/.icc 就能用。
/// 这样"支持多少种色彩空间"不再被硬编码列表限制 —— 放进对应配置文件即可生效。
/// 键为规范化后的空间名（小写、去掉 - . 空格），值为候选文件名片段（同样规范化）。
const SPACE_ALIASES: &[(&str, &[&str])] = &[
    ("srgb", &["srgbcolorspaceprofile", "srgb"]),
    ("p3", &["displayp3", "displayp3", "p3"]),
    ("dcip3", &["dcip3", "p3d65", "dci"]),
    ("adobergb", &["adobergb1998", "adobergb", "adobergb1998"]),
    ("rec709", &["rec709", "bt709", "itu709"]),
    ("rec2020", &["rec2020", "bt2020", "itu2020"]),
    ("prophoto", &["prophoto"]),
    ("gray", &["gray", "grey", "grayscale"]),
];

fn flatten_key(s: &str) -> String {
    s.to_lowercase()
        .replace("-", "")
        .replace(".", "")
        .replace(" ", "")
        .replace("_", "")
}

/// 定位目标 ICC 文件。
/// 顺序：系统内置 sRGB → 用户目录 %LOCALAPPDATA%\screenguard_icc\ → 系统色彩目录，
/// 按别名表模糊匹配文件名。找不到时给出可操作的指引（放哪个目录、从哪拷）。
fn icc_path(space: &str) -> Result<String, String> {
    let key = flatten_key(space);
    if key == "srgb" {
        let p = format!(r"{}\sRGB Color Space Profile.icm", SYSTEM_COLOR_DIR);
        if std::path::Path::new(&p).exists() {
            return Ok(p);
        }
    }
    let wants: Vec<String> = SPACE_ALIASES
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, a)| a.iter().map(|s| flatten_key(s)).collect())
        .unwrap_or_else(|| vec![key.clone()]);
    let user_dir = format!(
        "{}\\{}",
        std::env::var("LOCALAPPDATA").unwrap_or_default(),
        COLOR_DIR
    );
    for dir in [user_dir.clone(), SYSTEM_COLOR_DIR.to_string()] {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let low = name.to_lowercase();
            let ext = low.rsplit('.').next().unwrap_or("").to_string();
            if ext != "icm" && ext != "icc" {
                continue;
            }
            let flat = flatten_key(&low);
            if wants.iter().any(|w| !w.is_empty() && flat.contains(w.as_str())) {
                return Ok(e.path().to_string_lossy().to_string());
            }
        }
    }
    Err(format!(
        "缺少「{}」色彩配置文件：把对应的 .icc/.icm 放到 {} 后重试（Windows 只自带 sRGB；P3/AdobeRGB/Rec.2020 等可从 macOS 的 /System/Library/ColorSync/Profiles/ 拷贝，或从显示器厂商官网下载）",
        space, user_dir
    ))
}

/// C# 桥接源码的落盘路径（%TEMP%\screenguard_core.cs）
fn core_cs_file() -> std::path::PathBuf {
    std::env::temp_dir().join("screenguard_core.cs")
}

/// 确保 C# 桥接源码已落盘（带 UTF-8 BOM，PowerShell 5.1 才能正确解析中文注释），
/// 返回可传给 PowerShell 的正斜杠路径。
///
/// 为什么必须落盘而不是内联：整段 C# 内联进 `Add-Type -TypeDefinition '<CORE_CS>'`
/// 会撞 Windows 命令行长度上限 —— 实测 app 报「无法启动 powershell: 文件名或扩展名太长
/// (os error 206)」，导致显示器列表 / DDC / 旋转等所有依赖 SGCore 的命令集体失败。
/// 改为 Add-Type -Path 后命令行只剩一个短路径，后续再加代码也不会再撞线。
/// C# 桥接的编译产物（程序集）缓存路径
fn core_dll_file() -> std::path::PathBuf {
    std::env::temp_dir().join("screenguard_core.dll")
}

/// 确保 C# 桥接可用，返回可传给 PowerShell 的加载路径。
///
/// 落盘原因：整段 C# 内联进 `Add-Type -TypeDefinition '<CORE_CS>'` 会撞 Windows 命令行
/// 长度上限（app 报「文件名或扩展名太长 (os error 206)」，所有依赖 SGCore 的命令集体失败）。
/// 编译缓存原因：每次调用都 `Add-Type -Path '<源文件>'` 会**重新编译整份 C#**（实测 1~3 秒），
/// 拖亮度滑块时每一格都要等一次，界面直接卡死。改成先编译成程序集（仅在源码变化时做一次），
/// 之后每次只加载 DLL（约 0.2 秒）。
fn core_asm_ref() -> Result<String, String> {
    let cs = core_cs_file();
    let dll = core_dll_file();
    // 1) 源码落盘（带 UTF-8 BOM：PowerShell 5.1 没 BOM 会按 ANSI 读中文注释 → 编译失败）
    let want = CORE_CS.len() as u64 + 3; // + 3 字节 BOM
    if !matches!(std::fs::metadata(&cs), Ok(m) if m.len() == want) {
        let mut buf = Vec::with_capacity(CORE_CS.len() + 3);
        buf.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        buf.extend_from_slice(CORE_CS.as_bytes());
        std::fs::write(&cs, &buf).map_err(|e| format!("写入 C# 桥接文件失败：{}", e))?;
    }
    let cs_p = cs.display().to_string().replace('\\', "/");
    let dll_p = dll.display().to_string().replace('\\', "/");
    // 2) 程序集可用？存在、非空、且不旧于源文件
    let fresh = match (std::fs::metadata(&cs), std::fs::metadata(&dll)) {
        (Ok(c), Ok(d)) => match (c.modified(), d.modified()) {
            (Ok(cm), Ok(dm)) => dm >= cm && d.len() > 0,
            _ => false,
        },
        _ => false,
    };
    if fresh {
        return Ok(dll_p);
    }
    // 3) 编译（失败就退回源码直载：慢，但至少能用，不会因为编译问题整个功能不可用）
    let script = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\nAdd-Type -Path '{cs}' -OutputAssembly '{dll}'",
        cs = cs_p,
        dll = dll_p
    );
    if ps(&script).is_err() || !dll.exists() {
        return Ok(cs_p);
    }
    Ok(dll_p)
}

/// 在已加载 CORE_CS 的会话里执行一段桥接调用
fn ps_core(call: &str) -> Result<String, String> {
    let cs = core_asm_ref()?;
    let script = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\nAdd-Type -Path '{cs}';\n{call}",
        cs = cs,
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
    mark_change(); // 用户主动改的显示配置：自动修复 120 秒内不要插手
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

// ---------- 跨屏 DPI 对齐：真正落地（写 HKCU 逐屏缩放档）+ 可回滚 ----------
// 为什么旧版"看起来没实现"：Windows 改每屏缩放只有注册表一条路，且必须注销/重登才生效，
// 没有任何运行时 API 能立刻改 —— 所以旧版只做了诊断。现在改为「先备份、再统一、可还原」。

fn dpi_backup_path() -> String {
    format!(
        "{}\\Screenguard\\dpi_backup.txt",
        std::env::var("APPDATA").unwrap_or_default()
    )
}

/// 读出逐屏缩放档：返回 (注册表键名, DpiValue) 列表
fn dpi_rows() -> Result<Vec<(String, u32)>, String> {
    let script = "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\n\
                  $vals = Get-ChildItem 'HKCU:\\Control Panel\\Desktop\\PerMonitorSettings' -ErrorAction SilentlyContinue;\n\
                  foreach ($v in $vals) { $p = Get-ItemProperty $v.PSPath -ErrorAction SilentlyContinue; Write-Output ('S|' + $v.PSChildName + '|' + $p.DpiValue) }";
    let raw = ps(script).unwrap_or_default();
    let mut rows = Vec::new();
    for line in raw.lines() {
        let p: Vec<&str> = line.split('|').collect();
        if p.len() >= 3 && p[0] == "S" && !p[1].trim().is_empty() {
            rows.push((p[1].trim().to_string(), p[2].trim().parse::<u32>().unwrap_or(0)));
        }
    }
    if rows.is_empty() {
        return Err("读不到逐屏缩放设置（PerMonitorSettings 为空）".to_string());
    }
    Ok(rows)
}

fn dpi_write(key: &str, value: u32) -> Result<(), String> {
    // 键名含 ^ 与十六进制，一律走单引号字符串；不做任何转义处理以免破坏键名
    let s = format!(
        "$p = 'HKCU:\\Control Panel\\Desktop\\PerMonitorSettings\\{}'; if (-not (Test-Path $p)) {{ New-Item -Path $p -Force | Out-Null }}; Set-ItemProperty -Path $p -Name DpiValue -Value {} -Type DWord; Write-Output OK",
        key, value
    );
    let out = ps(&s)?;
    if out.contains("OK") {
        Ok(())
    } else {
        Err(format!("写入缩放档 {} 失败", key))
    }
}

/// 把副屏（以及其它屏）的缩放档统一成主屏的值；原值备份，可一键还原。
pub fn match_dpi_apply() -> Result<String, String> {
    mark_change(); // 用户主动改的显示配置：自动修复 120 秒内不要插手
    let displays = get_displays();
    if displays.len() < 2 {
        return Err("需要主屏 + 副屏各一块才能使用此功能".to_string());
    }
    let main = displays.iter().find(|d| d.main).ok_or("未找到主屏")?;
    let mkey = short_id(&main.id).to_lowercase();
    let rows = dpi_rows()?;
    let target = rows
        .iter()
        .find(|(k, _)| !mkey.is_empty() && k.to_lowercase().starts_with(&mkey))
        .map(|(_, v)| *v)
        .ok_or_else(|| {
            "没找到主屏对应的缩放档（两块屏可能被系统记在同一个键上）。请到「设置 → 屏幕 → 缩放」手动把两块屏设成同一百分比".to_string()
        })?;
    if target == 0 {
        return Err("主屏缩放当前是「跟随系统」，无法作为对齐目标：请先在系统设置里给主屏选一个具体百分比".to_string());
    }
    // 备份（每行 键名|原值）
    let mut buf = String::new();
    for (k, v) in &rows {
        buf.push_str(&format!("{}|{}\n", k, v));
    }
    let bp = dpi_backup_path();
    if let Some(dir) = std::path::Path::new(&bp).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&bp, &buf).map_err(|e| format!("备份缩放设置失败：{}", e))?;
    let mut changed = 0;
    for (k, v) in &rows {
        if *v == target {
            continue;
        }
        if dpi_write(k, target).is_ok() {
            changed += 1;
        }
    }
    Ok(format!(
        "已把 {} 个缩放档统一为 {}（原值已备份到 {}）。Windows 需注销或重新登录后生效；不满意可点「还原缩放」回到原样",
        changed,
        target,
        bp
    ))
}

/// 还原到 match_dpi_apply 之前备份的缩放档
pub fn match_dpi_restore() -> Result<String, String> {
    mark_change(); // 用户主动改的显示配置：自动修复 120 秒内不要插手
    let bp = dpi_backup_path();
    let txt = std::fs::read_to_string(&bp)
        .map_err(|_| "没有可还原的缩放备份（还没用「窗口跨屏等大」对齐过）".to_string())?;
    let mut n = 0;
    for line in txt.lines() {
        let p: Vec<&str> = line.split('|').collect();
        if p.len() < 2 || p[0].trim().is_empty() {
            continue;
        }
        let v: u32 = p[1].trim().parse().unwrap_or(0);
        if dpi_write(p[0].trim(), v).is_ok() {
            n += 1;
        }
    }
    Ok(format!("已把 {} 个缩放档还原为备份值（注销或重新登录后生效）", n))
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

// ---------- 「刚主动改过显示配置」闸门 ----------
// 为什么要它：切旋转/写缩放/铺满/HDR/色彩都会产生一堆显示事件，自动修复线程看到
// 就当成「显示器掉线」去修（强制重协商），于是把我们**有意**改的状态又改回去 ——
// 用户实测：点「副屏方向」当场生效、几秒后自己变回去了，正是这个原因。
static LAST_INTENT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

fn mark_change() {
    if let Ok(mut g) = LAST_INTENT.lock() {
        *g = Some(std::time::Instant::now());
    }
}

/// 最近 secs 秒内是否由用户主动改过显示配置（自动修复要让路）
pub fn recently_changed(secs: u64) -> bool {
    if let Ok(g) = LAST_INTENT.lock() {
        if let Some(t) = *g {
            return t.elapsed().as_secs() < secs;
        }
    }
    false
}

/// 横竖屏切换（再点一次转回）
pub fn rotate_secondary() -> Result<(), String> {
    mark_change(); // 用户主动改的显示配置：自动修复 120 秒内不要插手
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
    mark_change(); // 用户主动改的显示配置：自动修复 120 秒内不要插手
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

// ================= 屏幕电源控制（方案A：纯本地，不依赖网络） =================
// 设计要点：本机小米显示器（Redmi G Pro 27U）DDC/CI 不通，无法用 DDC 控制它；
// 因此「一次黑掉所有屏」走 Windows 系统级指令（对所有屏生效，含小米），
// 而单台精细待机/唤醒走 DDC（实测副屏 MTI 支持 VCP 0xD6）。

/// 系统级关闭所有显示器：一次让全部屏进入待机。
/// 这是唯一能影响到无 DDC 显示器（本机小米）的软件手段。
pub fn screen_off() -> Result<(), String> {
    let r = screen_off_inner();
    if r.is_ok() {
        // 记录「是**我们主动**关的屏」并同时记下此刻的输入时刻。
        // 必须这么做：待机会让系统产生「显示器掉线」事件，自动修复线程看到就会去修，
        // 而深度修复里含有 screen_off() —— 结果就是「鼠标一唤醒、屏又立刻黑下去」（用户实测）。
        *asleep_tick().lock().unwrap() = last_input_tick().max(1);
    }
    r
}

/// 是否处于「我们主动让屏幕待机」的状态。
/// 用户一有输入（鼠标/键盘）就自动判定为已唤醒并清除标记 —— 这样唤醒后不会再被关回去。
pub fn standby_state() -> bool {
    let off_at = *asleep_tick().lock().unwrap();
    if off_at == 0 {
        return false;
    }
    let li = last_input_tick();
    if li != 0 && li > off_at {
        // 用户已经动过键鼠 → 视为已唤醒，清掉标记
        *asleep_tick().lock().unwrap() = 0;
        return false;
    }
    true
}

fn screen_off_inner() -> Result<(), String> {
    let out = ps_core("[SGCore]::ScreenOff()")?;
    if out.trim() == "OK" {
        Ok(())
    } else {
        Err(format!("关闭屏幕失败：{}", out.trim()))
    }
}

/// 唤醒屏幕：合成一次 Shift 按键（真实输入事件，可解除系统级显示器待机）
pub fn screen_wake() -> Result<(), String> {
    let r = screen_wake_inner();
    // 唤醒（不论是我们发的还是用户动键鼠）→ 清掉「待机中」标记，自动修复恢复工作
    *asleep_tick().lock().unwrap() = 0;
    r
}

fn screen_wake_inner() -> Result<(), String> {
    let out = ps_core("[SGCore]::ScreenWake()")?;
    if out.trim() == "OK" {
        Ok(())
    } else {
        Err(format!("唤醒屏幕失败：{}", out.trim()))
    }
}

/// DDC 单台电源模式：1=开 2=待机 4=软关（实测唤醒后需 2~5 秒 DDC 才恢复响应）
pub fn set_display_power(display_id: &str, mode: u32) -> Result<(), String> {
    mark_change(); // 用户主动改的显示配置：自动修复 120 秒内不要插手
    vcp_op(display_id, 0xD6, Some(mode))
}

/// 读单台显示器电源模式（1=开 2=待机 4=软关 5=硬关）
pub fn get_display_power(display_id: &str) -> Result<u32, String> {
    let dev = resolve_dev(display_id)?;
    // 快路径：原生读 VCP 0xD6（毫秒级）
    if let Some(v) = ddc_get(&dev, 0xD6) {
        return Ok(v);
    }
    // 慢路径：原 PowerShell 实现兜底
    let out = ps_core(&format!("[SGCore]::DDCPowerRead('{dev}')", dev = dev))?;
    let t = out.trim();
    if t == "ERR" {
        return Err("该显示器不支持电源模式控制（VCP 0xD6）".to_string());
    }
    t.parse::<u32>()
        .map_err(|_| format!("电源模式读取异常：{}", t))
}

/// 显示器在线快照（轻量，供联动检测轮询用）：返回当前在线显示器的稳定标识列表。
/// 与 get_displays 的区别：不做 DDC 探测与名字查询，只枚举，适合 2~3 秒级轮询。
pub fn display_snapshot() -> Vec<String> {
    let raw = match ps_core("[SGCore]::ListDisplays()") {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for line in raw.lines() {
        let p: Vec<&str> = line.split('|').collect();
        if p.len() >= 3 && p[0] == "D" {
            out.push(p[2].trim().to_string());
        }
    }
    out.sort();
    out
}

// ================= 显示器健康：KVM 切换 / EDID 重新协商异常检测与修复 =================
// 背景：本机显示链路是「电脑 --USB-C→DP/HDMI-- KVM --DP/HDMI-- 两块屏」。
// KVM 在「切换电脑 / 显示器开关 / 电脑重启」时会重新协商 EDID，协商失败表现为
// ①黑屏不恢复 ②抖动花屏 ③清晰度变差、画面发白发亮（色彩格式被降级成 YCbCr）。
// 这些是「显示配置状态错了」而不是硬件坏 —— 重置显示配置即可恢复（所以更新显卡驱动
// 有时也能好）。本模块只做两件事：把异常事件读出来（供日志与提醒），以及一键重新协商。

/// 读取最近的显示链路异常事件。每行：E|时间|等级|来源|事件ID|摘要
pub fn health_events(minutes: u32) -> String {
    let csp = match core_asm_ref() {
        Ok(p) => p,
        Err(_) => return String::new(),
    };
    let script = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\nAdd-Type -Path '{cs}';\n[SGCore]::ReadDisplayEvents({m})",
        cs = csp,
        m = minutes
    );
    ps(&script).unwrap_or_default()
}

/// 每台在用屏幕的当前模式。每行：M|设备名|宽|高|刷新率|方向
pub fn mode_detail() -> String {
    let csp = match core_asm_ref() {
        Ok(p) => p,
        Err(_) => return String::new(),
    };
    let script = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\nAdd-Type -Path '{cs}';\n[SGCore]::ModeDetail()",
        cs = csp
    );
    ps(&script).unwrap_or_default()
}

/// 一键修复显示异常（分级，从轻到重）：
///   level 1 —— 对每台屏强制重新协商（先降刷新率再切回原模式，不改变任何用户设置）；
///   level 2 —— 额外做一次系统级关屏+唤醒，强制整条链路重新握手，然后再协商一轮。
/// 返回逐台结果，每行：<设备名>|<结果>
pub fn repair_display(level: u32) -> Result<String, String> {
    let csp = core_asm_ref()?;
    let renegotiate = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\nAdd-Type -Path '{cs}';\n$ds = @();\nforeach ($m in ([SGCore]::ModeDetail().Split([char]10))) {{ $p = $m.Split([char]124); if ($p[0] -eq 'M') {{ $ds += $p[1] }} }};\nforeach ($d in $ds) {{ Write-Output ('R|' + $d + '|' + [SGCore]::ForceReNegotiate($d)) }}",
        cs = csp
    );

    let mut out: Vec<String> = Vec::new();
    let first = ps(&renegotiate).unwrap_or_default();
    let mut count = 0;
    for line in first.lines() {
        let p: Vec<&str> = line.split('|').collect();
        if p.len() >= 3 && p[0] == "R" {
            out.push(format!("{}|{}", p[1], p[2..].join("|")));
            count += 1;
        }
    }
    if count == 0 {
        return Err("未枚举到可用显示器，修复未执行".to_string());
    }

    if level >= 2 {
        // 更强：系统级关屏再唤醒（强制整条链路重新握手），然后重跑一轮协商
        let _ = screen_off();
        std::thread::sleep(std::time::Duration::from_millis(1200));
        let _ = screen_wake();
        std::thread::sleep(std::time::Duration::from_millis(1800));
        let second = ps(&renegotiate).unwrap_or_default();
        let mut n2 = 0;
        for line in second.lines() {
            let p: Vec<&str> = line.split('|').collect();
            if p.len() >= 3 && p[0] == "R" {
                out.push(format!("重协商-2 {}|{}", p[1], p[2..].join("|")));
                n2 += 1;
            }
        }
        out.push(format!(
            "深度修复|已强制链路重握手（关屏+唤醒），二次协商 {}/{} 台成功",
            n2, count
        ));
    }

    Ok(out.join("\n"))
}

// ================= 方案B：ADB 联网精细控制（可选增强） =================
// 前提：显示器开启「网络 ADB 调试」（一次性设置，类似手机开 USB 调试）。
// 找不到 adb 不影响方案A；找到后即可：遥控器按键实时联动、精确控制电源/音量/输入源。

use std::os::windows::process::CommandExt;

/// 探测 adb 可执行文件：程序同目录 → 我们的下载目录 → Android SDK → PATH
pub fn adb_exe() -> Option<std::path::PathBuf> {
    let mut cands: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.push(dir.join("platform-tools").join("adb.exe"));
            cands.push(dir.join("adb.exe"));
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let l = std::path::PathBuf::from(&local);
        cands.push(l.join("screenguard_adb").join("platform-tools").join("adb.exe"));
        cands.push(l.join("Android").join("Sdk").join("platform-tools").join("adb.exe"));
    }
    // 本机已装工具自带的 adb：ASUS GlideX（ROG 机型预装，实测 1.0.41 可用）、Android SDK、scrcpy 等
    for p in [
        r"C:\Program Files\ASUS\GlideX\adb.exe",
        r"C:\Program Files (x86)\ASUS\GlideX\adb.exe",
        r"C:\Program Files\Android\platform-tools\adb.exe",
        r"C:\platform-tools\adb.exe",
    ] {
        cands.push(std::path::PathBuf::from(p));
    }
    for c in cands.iter() {
        if c.exists() {
            return Some(c.clone());
        }
    }
    None
}

/// 执行 adb 子命令（不弹黑窗）
fn adb_cmd(exe: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(exe)
        .args(args)
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| format!("adb 执行失败：{}", e))?;
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let e = String::from_utf8_lossy(&out.stderr).to_string();
    if out.status.success() {
        Ok(s)
    } else {
        Err(format!("{}{}", s, e).trim().to_string())
    }
}

/// ADB 能力探测：是否找到 adb、已连接哪些设备
pub fn adb_status() -> super::AdbStatus {
    match adb_exe() {
        None => super::AdbStatus {
            available: false,
            path: String::new(),
            devices: Vec::new(),
            tip: "未找到 adb：方案B 的联网精细控制需要 Android Platform Tools（点「下载 ADB」自动获取）。方案A 本地功能不受影响。".to_string(),
        },
        Some(exe) => {
            let devices: Vec<String> = adb_cmd(&exe, &["devices"])
                .map(|s| {
                    s.lines()
                        .skip(1)
                        .filter_map(|l| {
                            let t = l.trim();
                            if t.is_empty() || t.starts_with('*') {
                                return None;
                            }
                            t.split('\t').next().map(|x| x.trim().to_string())
                        })
                        .filter(|x| !x.is_empty() && !x.contains("offline"))
                        .collect()
                })
                .unwrap_or_default();
            super::AdbStatus {
                available: true,
                path: exe.display().to_string(),
                devices,
                tip: "adb 就绪".to_string(),
            }
        }
    }
}

/// 连接显示器的 ADB（addr 形如 192.168.31.216:5555）
pub fn adb_connect(addr: &str) -> Result<String, String> {
    let exe = adb_exe().ok_or("未找到 adb，请先点「下载 ADB」")?;
    adb_cmd(&exe, &["connect", addr])
}

/// 在指定设备上执行 shell 命令
pub fn adb_shell(serial: &str, cmd: &str) -> Result<String, String> {
    let exe = adb_exe().ok_or("未找到 adb，请先点「下载 ADB」")?;
    adb_cmd(&exe, &["-s", serial, "shell", cmd])
}

/// 电源键（KEYCODE_POWER = 26）：待机/唤醒
pub fn adb_power(serial: &str) -> Result<String, String> {
    adb_shell(serial, "input keyevent 26")
}

/// 音量（KEYCODE_VOLUME_UP = 24 / VOLUME_DOWN = 25）
pub fn adb_volume(serial: &str, up: bool) -> Result<String, String> {
    adb_shell(serial, if up { "input keyevent 24" } else { "input keyevent 25" })
}

/// 一键下载 Android Platform Tools 到 %LOCALAPPDATA%\screenguard_adb（零依赖：PowerShell 下载 + 解压）
pub fn adb_download() -> Result<String, String> {
    let script = r#"$ErrorActionPreference='Stop';
$dst = Join-Path $env:LOCALAPPDATA 'screenguard_adb';
New-Item -ItemType Directory -Force -Path $dst | Out-Null;
$zip = Join-Path $dst 'pt.zip';
$urls = @('https://dl.google.com/android/repository/platform-tools-latest-windows.zip','https://googledownloads.cn/android/repository/platform-tools-latest-windows.zip');
$ok = $false;
foreach ($u in $urls) { try { Invoke-WebRequest -Uri $u -OutFile $zip -TimeoutSec 300 -UseBasicParsing; $ok = $true; break } catch { } }
if (-not $ok) { Write-Output 'ERR:下载失败（网络不通），请手动下载 platform-tools 解压到该目录'; exit }
Expand-Archive -Path $zip -DestinationPath $dst -Force;
Remove-Item $zip -Force;
Write-Output ('OK:' + (Join-Path $dst 'platform-tools\adb.exe'))"#;
    let out = ps(script)?;
    let t = out.trim();
    if t.starts_with("OK:") {
        Ok(t[3..].to_string())
    } else if t.starts_with("ERR:") {
        Err(t[4..].to_string())
    } else {
        Err(t.to_string())
    }
}

// ================= 软件层总亮度（gamma ramp，对所有屏生效） =================
// 为什么用 gamma 而不是 DDC：本机小米显示器经 KVM 的 DP 链路不转发 DDC/CI
// （Mac 直连可读），DDC 读不到亮度/音量；gamma ramp 由 GDI 直接作用于显卡输出，
// 不依赖链路协议，因此对两块屏（含小米）都生效。
// 安全：三通道同曲线（不改色相）、范围锁 40~160、不写注册表（重启即失效）。

/// 各屏当前总亮度：行 G|<dev>|<pct>
pub fn gamma_get() -> String {
    ps_core("[SGCore]::GammaGetAll()").unwrap_or_default()
}

/// 设置所有屏的总亮度（40~160，100 = 原始）
pub fn gamma_set(pct: u32) -> Result<String, String> {
    let p = pct.clamp(40, 160);
    let out = ps_core(&format!("[SGCore]::GammaSetAll({})", p))?;
    let ok = out.lines().filter(|l| l.starts_with("OK|")).count();
    let fail = out.lines().filter(|l| l.starts_with("FAIL|")).count();
    if ok == 0 && fail == 0 {
        return Err("没有找到可调亮度的在用显示器".to_string());
    }
    Ok(format!("{} 台已应用{}", ok, if fail > 0 { format!("，{} 台失败", fail) } else { String::new() }))
}

// ================= HDR 同步 =================

/// 各屏 HDR 状态：行 H|<dev>|<支持>|<已启用>
pub fn hdr_states() -> String {
    ps_core("[SGCore]::HDRStates()").unwrap_or_default()
}

/// 把所有支持 HDR 的屏统一设为开/关
pub fn hdr_set(on: bool) -> Result<String, String> {
    let out = ps_core(&format!("[SGCore]::HDRSetAll({})", if on { 1 } else { 0 }))?;
    let ok = out.lines().filter(|l| l.contains("|OK")).count();
    let skip = out.lines().filter(|l| l.contains("SKIP")).count();
    let err: Vec<&str> = out.lines().filter(|l| l.contains("|ERR")).collect();
    if ok == 0 && skip > 0 && err.is_empty() {
        return Err("本机没有支持 HDR 的显示器".to_string());
    }
    let mut msg = format!("{} 台已切换", ok);
    if out.contains("apply=ERR") {
        msg.push_str("（注意：Apply 步骤失败 → HDR 可能并未真正生效）");
    }
    if skip > 0 {
        msg.push_str(&format!("，{} 台不支持 HDR 已跳过", skip));
    }
    if !err.is_empty() {
        msg.push_str(&format!("，{} 台失败", err.len()));
    }
    // 把逐台明细一并回传：API 返回成功 != 屏幕真的切换了（这台机上实测过），
    // 与其给一个漂亮但可能不实的结论，不如让用户看到原始结果。
    let detail: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if !detail.is_empty() {
        msg.push_str(&format!("｜明细：{}", detail));
    }
    Ok(msg)
}

// ================= 双屏铺满（任意前台窗口） =================

/// 铺满状态文件（与播放器铺满分开，互不干扰）
fn span_fg_file() -> String {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    format!(r"{}\screenguard_span_fg.json", base)
}

/// 把当前前台窗口铺满所有屏（浏览器里的抖音全屏、播放器、任意窗口都可以）
/// mode: 0 = 铺满整块桌面包围盒；1 = 以主屏为基准（高度取主屏，避免主屏底部被裁）
pub fn span_foreground(mode: u32) -> Result<String, String> {
    mark_change(); // 用户主动改的显示配置：自动修复 120 秒内不要插手
    let out = ps_core(&format!(
        "[SGCore]::SpanForeground('{}', {}, {})",
        span_fg_file(),
        std::process::id(),
        mode
    ))?;
    let t = out.trim();
    if let Some(rest) = t.strip_prefix("OK|") {
        return Ok(rest.to_string());
    }
    Err(t.strip_prefix("ERR:").unwrap_or(t).to_string())
}

/// 还原上一次被铺满的窗口
pub fn restore_foreground() -> Result<(), String> {
    let out = ps_core(&format!("[SGCore]::RestoreForeground('{}')", span_fg_file()))?;
    let t = out.trim();
    if t == "OK" {
        Ok(())
    } else {
        Err(t.strip_prefix("ERR:").unwrap_or(t).to_string())
    }
}

// ================= 全局快捷键（原生 RegisterHotKey，零额外依赖） =================
// 用户要求：不能用小米遥控器，就固定一个电脑快捷键，并在软件界面里显示出来。
// 固定键位：Ctrl+Alt+L —— 切换「全部屏幕待机 / 唤醒」。
// 实现：独立线程注册（NULL hwnd → 消息投递到线程队列）+ GetMessage 消息循环。
// 判断当前该关还是该开：记录「屏幕待机」时刻，配合 GetLastInputInfo——
// 若此后没有任何输入（鼠标/键盘），说明屏还是关着的 → 本次按就是唤醒；
// 若已有输入（用户自己动鼠标唤醒了）→ 本次按就是再关掉。

const WM_HOTKEY: u32 = 0x0312;
const MOD_ALT: u32 = 0x0001;
const MOD_CONTROL: u32 = 0x0002;
const MOD_NOREPEAT: u32 = 0x4000;
const VK_L: u32 = 0x4C;

#[repr(C)]
struct SgMsg {
    hwnd: *mut std::ffi::c_void,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt_x: i32,
    pt_y: i32,
    l_private: u32,
}

#[repr(C)]
struct SgLastInput {
    cb_size: u32,
    dw_time: u32,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterHotKey(hwnd: *mut std::ffi::c_void, id: i32, modifiers: u32, vk: u32) -> i32;
    fn GetMessageW(msg: *mut SgMsg, hwnd: *mut std::ffi::c_void, min: u32, max: u32) -> i32;
    fn GetLastInputInfo(pli: *mut SgLastInput) -> i32;
}

fn hotkey_status() -> &'static Mutex<(bool, String)> {
    static S: OnceLock<Mutex<(bool, String)>> = OnceLock::new();
    S.get_or_init(|| Mutex::new((false, String::new())))
}

/// 屏幕待机时刻（GetTickCount 毫秒）；0 = 当前认为屏是亮的
fn asleep_tick() -> &'static Mutex<u32> {
    static S: OnceLock<Mutex<u32>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(0))
}

fn last_input_tick() -> u32 {
    unsafe {
        let mut li = SgLastInput { cb_size: std::mem::size_of::<SgLastInput>() as u32, dw_time: 0 };
        if GetLastInputInfo(&mut li) != 0 {
            li.dw_time
        } else {
            0
        }
    }
}

/// 热键动作：切换全部屏幕待机 / 唤醒
fn hotkey_toggle() {
    let off_at = *asleep_tick().lock().unwrap();
    let li = last_input_tick();
    // off_at != 0 且此后没有新输入 → 屏仍处于我们关掉的状态 → 唤醒
    let should_wake = off_at != 0 && (li == 0 || li <= off_at);
    if should_wake {
        let _ = screen_wake();
        *asleep_tick().lock().unwrap() = 0;
    } else {
        let _ = screen_off();
        *asleep_tick().lock().unwrap() = last_input_tick().max(1);
    }
}

/// 注册全局快捷键（幂等：已注册则直接返回成功）。成功返回 "Ctrl+Alt+L"
pub fn hotkey_start() -> Result<String, String> {
    {
        let g = hotkey_status().lock().unwrap();
        if g.0 {
            return Ok("Ctrl+Alt+L".to_string());
        }
    }
    std::thread::spawn(|| unsafe {
        let id = 0x5347; // SG：Ctrl+Alt+L 全部屏幕待机 / 唤醒
        let id2 = 0x5348; // SG2：Ctrl+Alt+R 还原被铺满的窗口（不依赖界面，防止铺满后点不到按钮）
        if RegisterHotKey(std::ptr::null_mut(), id, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_L) == 0 {
            return; // 注册失败：状态保持 (false, "")，由调用方读到失败
        }
        // 第二个键失败不影响第一个键（例如被别的软件占用）
        RegisterHotKey(std::ptr::null_mut(), id2, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 0x52);
        {
            let mut g = hotkey_status().lock().unwrap();
            g.0 = true;
            g.1 = "Ctrl+Alt+L".to_string();
        }
        let mut m: SgMsg = std::mem::zeroed();
        while GetMessageW(&mut m, std::ptr::null_mut(), 0, 0) > 0 {
            if m.message == WM_HOTKEY {
                if m.wparam == id as usize {
                    hotkey_toggle();
                } else if m.wparam == id2 as usize {
                    // 铺满之后的窗口可能盖住界面上的「还原」按钮，
                    // 所以必须有一个不依赖界面的还原入口
                    let _ = restore_foreground();
                }
            }
        }
    });
    std::thread::sleep(std::time::Duration::from_millis(160));
    let g = hotkey_status().lock().unwrap();
    if g.0 {
        Ok(g.1.clone())
    } else {
        Err("快捷键 Ctrl+Alt+L 注册失败（可能已被其它程序占用）".to_string())
    }
}

/// 当前生效的全局快捷键（空串 = 未注册）
pub fn hotkey_label() -> String {
    hotkey_status().lock().map(|g| g.1.clone()).unwrap_or_default()
}

// ================= v0.3.3：窗口选择 / 音频端点 / 状态快照与一键恢复默认 =================
// 这部分的由来（用户逐条要求）：
//  ① 「铺满」要能**让用户选哪个窗口**，铺满后要**方便还原**（铺满自己会找不到按钮）；
//  ② 凡是**软件层面**的改动（总亮度 gamma、HDR、色彩、窗口铺满、音量）都要能
//     **一键回到默认/原始状态**，避免用户乱调之后不知道该恢复成什么值；
//  ③ **退出软件时自动还原**成原样。
// 做法：应用启动时采集一次「基线快照」存 %APPDATA%\Screenguard\state.json
// （gamma 视作 100=原始曲线；HDR/ICC/音量记录当时真实值），
// 之后「恢复默认」「退出还原」都回到这份基线。硬件层面（显示器自己的亮度/音量、KVM）不动。

fn app_state_file() -> String {
    format!(
        "{}\\Screenguard\\state.json",
        std::env::var("APPDATA").unwrap_or_default()
    )
}

fn def_true() -> bool {
    true
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct AppState {
    /// 用户选定的输出端点 id（空 = 跟随系统默认）
    #[serde(default)]
    pub audio_dev: String,
    /// 退出软件时自动还原所有软件层面改动
    #[serde(default = "def_true")]
    pub revert_on_exit: bool,
    #[serde(default)]
    pub baseline_taken: bool,
    /// "设备名|0或1"
    #[serde(default)]
    pub base_hdr: Vec<String>,
    /// "设备名|ICC 路径"
    #[serde(default)]
    pub base_icc: Vec<String>,
    #[serde(default)]
    pub base_vol_id: String,
    #[serde(default)]
    pub base_vol: u32,
    #[serde(default)]
    pub base_mute: bool,
    /// 各屏当前旋转角度（"设备|角度"），供「恢复默认」把副屏方向切回原样
    #[serde(default)]
    pub base_rot: Vec<String>,
    /// 小米屏基线音量（0 = 未记录/不可用）
    #[serde(default)]
    pub base_mitv: u32,
    /// 小米显示器 MiTV Assistant 的地址（空 = 用默认）
    #[serde(default)]
    pub mitv_ip: String,
}

pub fn app_state() -> AppState {
    std::fs::read_to_string(app_state_file())
        .ok()
        .and_then(|s| serde_json::from_str::<AppState>(&s).ok())
        .unwrap_or_default()
}

pub fn save_app_state(st: &AppState) -> Result<(), String> {
    let p = app_state_file();
    if let Some(d) = std::path::Path::new(&p).parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let js = serde_json::to_string_pretty(st).map_err(|e| e.to_string())?;
    std::fs::write(&p, js).map_err(|e| format!("写入状态文件失败：{}", e))
}

pub fn set_revert_on_exit(on: bool) -> Result<(), String> {
    let mut st = app_state();
    st.revert_on_exit = on;
    save_app_state(&st)
}

pub fn app_state_json() -> String {
    let st = app_state();
    serde_json::json!({
        "revert_on_exit": st.revert_on_exit,
        "audio_dev": st.audio_dev,
        "baseline_taken": st.baseline_taken,
        "base_hdr": st.base_hdr,
        "base_vol": st.base_vol,
        "base_vol_id": st.base_vol_id,
    })
    .to_string()
}

/// 采集基线（启动时调用一次）。已有基线不覆盖 —— 否则第二次启动会把
/// 「上次改过的状态」当成本次原始值，越还原越偏。
pub fn capture_baseline() -> Result<String, String> {
    let mut st = app_state();
    if st.baseline_taken {
        // 基线只在首启采一次（否则第二次启动会把「上次改过的状态」当原始值）。
        // 但新增字段（旋转方向 / 小米音量）在老版本装机上是空的 —— 这里做一次**补齐**：
        // 只填空缺的字段，不动已有的 HDR / ICC / 音量基线。
        let mut filled = false;
        if st.base_rot.is_empty() {
            let raw = mode_detail();
            for line in raw.lines() {
                let p: Vec<&str> = line.split('|').collect();
                if p.len() >= 6 && p[0] == "M" {
                    st.base_rot.push(format!("{}|{}", p[1], p[5].trim()));
                }
            }
            filled = true;
        }
        if st.base_mitv == 0 {
            if let Ok(v) = mitv_volume_get() {
                st.base_mitv = v;
                filled = true;
            }
        }
        if filled {
            save_app_state(&st)?;
            return Ok("基线已存在，补齐了新增字段".to_string());
        }
        return Ok("基线已存在".to_string());
    }
    st.base_hdr = hdr_states()
        .lines()
        .filter(|l| l.starts_with("H|"))
        .filter_map(|l| {
            let p: Vec<&str> = l.split('|').collect();
            if p.len() >= 4 {
                Some(format!("{}|{}", p[1], p[3]))
            } else {
                None
            }
        })
        .collect();
    st.base_icc = Vec::new();
    for d in get_displays() {
        if let Ok(dev) = resolve_dev(&d.id) {
            if let Some(path) = icc_get(&dev) {
                st.base_icc.push(format!("{}|{}", dev, path));
            }
        }
    }
    st.base_vol_id = st.audio_dev.clone();
    // 各屏旋转角度 + 小米屏音量：这两项也是「软件改动」，一并记入基线，
    // 这样「一键恢复默认」才能真的把所有软件层改动都还原（用户要求）。
    st.base_rot = Vec::new();
    {
        // 注意：Windows 侧 mode_detail() 直接返回 String（不是 Result）
        let raw = mode_detail();
        for line in raw.lines() {
            let p: Vec<&str> = line.split('|').collect();
            if p.len() >= 6 && p[0] == "M" {
                st.base_rot.push(format!("{}|{}", p[1], p[5].trim()));
            }
        }
    }
    st.base_mitv = mitv_volume_get().unwrap_or(0);
    if let Ok(eps) = audio_endpoints() {
        let pick = eps
            .iter()
            .find(|e| !st.audio_dev.is_empty() && e.id == st.audio_dev)
            .or_else(|| eps.iter().find(|e| e.adjustable))
            .or_else(|| eps.first());
        if let Some(e) = pick {
            st.base_vol_id = e.id.clone();
            st.base_vol = e.volume;
            st.base_mute = e.mute;
        }
    }
    st.baseline_taken = true;
    save_app_state(&st)?;
    Ok(format!(
        "基线已采集：HDR {} 项 / ICC {} 项 / 音量 {}%",
        st.base_hdr.len(),
        st.base_icc.len(),
        st.base_vol
    ))
}

/// 一键恢复默认 = 回到基线快照（软件层面的改动全部撤销）
pub fn restore_defaults() -> Result<String, String> {
    let st = app_state();
    let mut done: Vec<String> = Vec::new();
    let mut warn: Vec<String> = Vec::new();

    if gamma_set(100).is_ok() {
        done.push("总亮度 → 100%（原始曲线）".to_string());
    }
    if restore_foreground().is_ok() {
        done.push("已还原被铺满的窗口".to_string());
    }
    let _ = screen_wake();

    // ---- 以下都是「软件层做过的改动」，一律回到基线（用户要求：一键恢复默认要全都能还原）----

    // ① 跨屏缩放对齐（写进 HKCU PerMonitorSettings 的那个）
    match match_dpi_restore() {
        Ok(o) => done.push(format!("缩放对齐已还原（{}）", o.chars().take(28).collect::<String>())),
        Err(e) => warn.push(format!("缩放对齐: {}", e)),
    }
    // ② 各屏旋转角度（副屏横竖屏）
    for ent in st.base_rot.clone() {
        let p: Vec<&str> = ent.splitn(2, '|').collect();
        if p.len() < 2 || p[1].trim().is_empty() {
            continue;
        }
        let call = format!("[SGCore]::Rotate('{}',[uint32]{})", p[0], p[1].trim());
        match ps_core(&call) {
            Ok(o) if o.trim().starts_with("OK") => done.push(format!("屏幕方向 {} 已还原", p[0])),
            Ok(o) => warn.push(format!("屏幕方向 {}: {}", p[0], o.trim())),
            Err(e) => warn.push(format!("屏幕方向 {}: {}", p[0], e)),
        }
    }
    // ③ 小米屏音量（走它自己的系统，也是软件改动）
    if st.base_mitv > 0 {
        match mitv_volume_set(st.base_mitv) {
            Ok(v) => done.push(format!("小米音量 → {}%", v)),
            Err(e) => warn.push(format!("小米音量: {}", e)),
        }
    }
    // ④ 音频「控制的设备」回到基线那一个
    if !st.base_vol_id.is_empty() {
        if set_audio_device(&st.base_vol_id).is_ok() {
            done.push("音频控制设备已还原".to_string());
        }
    }

    for ent in st.base_hdr.clone() {
        let p: Vec<&str> = ent.splitn(2, '|').collect();
        if p.len() < 2 {
            continue;
        }
        let want = p[1].trim() == "1";
        match hdr_set_dev(p[0], want) {
            Ok(_) => done.push(format!("HDR {} → {}", p[0], if want { "开" } else { "关" })),
            Err(e) => warn.push(format!("HDR {}: {}", p[0], e)),
        }
    }
    for ent in st.base_icc.clone() {
        let p: Vec<&str> = ent.splitn(2, '|').collect();
        if p.len() < 2 {
            continue;
        }
        match ps_core(&format!(
            "[SGCore]::IccSetPath('{}', '{}')",
            p[0], p[1]
        )) {
            Ok(o) if o.trim().starts_with("OK") => {
                done.push(format!("色彩配置 {} 已还原", p[0]));
            }
            Ok(o) => warn.push(format!("色彩 {}: {}", p[0], o.trim())),
            Err(e) => warn.push(format!("色彩 {}: {}", p[0], e)),
        }
    }
    if st.base_vol > 0 || !st.base_vol_id.is_empty() {
        if audio_set_on(&st.base_vol_id, st.base_vol).is_ok() {
            done.push(format!("音量 → {}%", st.base_vol));
        }
    }

    let mut msg = format!("已恢复默认：{}", done.join("；"));
    if !warn.is_empty() {
        msg.push_str(&format!("。未完成：{}", warn.join("；")));
    }
    Ok(msg)
}

// ---------- 窗口列表 / 指定窗口铺满 ----------

/// 可被铺满的窗口列表（"hwnd|宽x高|标题"）
pub fn list_windows() -> Result<String, String> {
    let out = ps_core(&format!("[SGCore]::ListWindows({})", std::process::id()))?;
    let t = out.trim();
    if t.starts_with("ERR:") {
        Err(t[4..].to_string())
    } else {
        Ok(t.to_string())
    }
}

/// 铺满指定窗口（hwnd）；mode 含义同 span_foreground
pub fn span_window(hwnd: i64, mode: u32) -> Result<String, String> {
    mark_change(); // 用户主动改的显示配置：自动修复 120 秒内不要插手
    let out = ps_core(&format!(
        "[SGCore]::SpanWindow([int64]{}, '{}', {}, {})",
        hwnd,
        span_fg_file(),
        std::process::id(),
        mode
    ))?;
    let t = out.trim();
    if let Some(rest) = t.strip_prefix("OK|") {
        Ok(rest.to_string())
    } else if let Some(rest) = t.strip_prefix("ERR:") {
        Err(rest.to_string())
    } else {
        Err(t.to_string())
    }
}

// ---------- 音频输出端点 ----------

/// 列出所有活动的输出端点（Windows：CoreAudio）
pub fn audio_endpoints() -> Result<Vec<AudioEndpoint>, String> {
    let out = ps_core("[SGAudio]::AudioList()")?;
    let sel = app_state().audio_dev;
    let mut list = Vec::new();
    for line in out.lines() {
        let t = line.trim_end();
        let rest = match t.strip_prefix("A|") {
            Some(r) => r,
            None => continue,
        };
        let p: Vec<&str> = rest.split('|').collect();
        if p.len() < 6 {
            continue;
        }
        let id = p[0].trim().to_string();
        list.push(AudioEndpoint {
            selected: !sel.is_empty() && sel == id,
            id,
            name: p[1].trim().to_string(),
            volume: p[2].trim().parse().unwrap_or(0),
            mute: p[3].trim() == "1",
            adjustable: p[4].trim() == "1",
            form_factor: p[5].trim().parse().unwrap_or(0),
            is_default: p.len() > 6 && p[6].trim() == "1",
        });
    }
    if list.is_empty() {
        return Err(out.trim().to_string());
    }
    Ok(list)
}

fn audio_set_on(id: &str, v: u32) -> Result<(), String> {
    let out = ps_core(&format!(
        "[SGAudio]::AudioSetDev('{}', [uint32]{})",
        id.replace('\'', ""),
        v.min(100)
    ))?;
    let t = out.trim();
    if t == "OK" {
        Ok(())
    } else {
        Err(t.trim_start_matches("ERR:").to_string())
    }
}

/// 选定要控制的输出端点（空串 = 跟随系统默认）
pub fn set_audio_device(id: &str) -> Result<(), String> {
    let mut st = app_state();
    st.audio_dev = id.to_string();
    save_app_state(&st)
}

/// 设置音量：优先作用于「用户选定的端点」，没选就作用于系统默认端点
pub fn set_system_volume_sel(v: u32) -> Result<(), String> {
    let id = app_state().audio_dev;
    audio_set_on(&id, v)
}

/// 静音：同样遵循用户选定的端点
pub fn set_system_mute_sel(on: bool) -> Result<(), String> {
    let id = app_state().audio_dev;
    let out = ps_core(&format!(
        "[SGAudio]::AudioMuteDev('{}', [uint32]{})",
        id.replace('\'', ""),
        if on { 1 } else { 0 }
    ))?;
    let t = out.trim();
    if t == "OK" {
        Ok(())
    } else {
        Err(t.trim_start_matches("ERR:").to_string())
    }
}

/// 单台屏 HDR 开关（基线还原用）
pub fn hdr_set_dev(dev: &str, on: bool) -> Result<String, String> {
    mark_change(); // 用户主动改的显示配置：自动修复 120 秒内不要插手
    let out = ps_core(&format!(
        "[SGCore]::HDRSetDev('{}', {})",
        dev.replace('\'', ""),
        if on { 1 } else { 0 }
    ))?;
    let t = out.trim();
    if t.starts_with("OK") || t.starts_with("SKIP") {
        Ok(t.to_string())
    } else {
        Err(t.trim_start_matches("ERR:").to_string())
    }
}

// ---------- 小米显示器（REDMI G Pro 27U）：MiTV Assistant 音量 ----------
// 背景：小米这块屏不支持 DDC/CI（实测换过 Type-C 口、也换过 Intel/NVIDIA 分支，都不通），
// 所以它的**亮度**只能走软件层 gamma（见「总亮度」）。但它的**音量另有一条路**：
// 显示器本体是 Android(hyperOS)，6095 端口跑着 MiTV Assistant，实测：
//   GET /controller?action=getvolume                    -> {"data":{"volume":10,...}}
//   GET /controller?action=keyevent&keycode=volumeup    -> 每次 +1（实测 10→11）
//   GET /controller?action=keyevent&keycode=volumedown  -> 每次 -1（实测 11→10）
// 没有 setvolume（404），只能按键步进 → 用「读-步进-回读」逼近目标值。
// 只动音量键，不动电源/其它键；每次步进后回读校验，异常立即停止。

const MITV_IP_DEFAULT: &str = "192.168.31.216";

fn mitv_ip() -> String {
    let ip = app_state().mitv_ip;
    if ip.trim().is_empty() {
        MITV_IP_DEFAULT.to_string()
    } else {
        ip.trim().to_string()
    }
}

fn mitv_get(action: &str) -> Result<String, String> {
    let url = format!("http://{}:6095/controller?action={}", mitv_ip(), action);
    let script = format!(
        "$ProgressPreference='SilentlyContinue';try{{(Invoke-WebRequest -UseBasicParsing -TimeoutSec 5 -Uri '{}').Content}}catch{{'ERR:'+$_.Exception.Message}}",
        url
    );
    let out = ps(&script)?;
    let t = out.trim();
    if t.is_empty() {
        return Err("显示器无响应（确认显示器已联网且与电脑同网段）".to_string());
    }
    if let Some(rest) = t.strip_prefix("ERR:") {
        return Err(format!(
            "连接显示器失败：{}",
            rest.chars().take(90).collect::<String>()
        ));
    }
    Ok(t.to_string())
}

fn mitv_volume_parse(body: &str) -> Option<u32> {
    let key = "\"volume\"";
    let i = body.find(key)? + key.len();
    let rest = &body[i..];
    let c = rest.find(':')? + 1;
    let digits: String = rest[c..]
        .chars()
        .skip_while(|ch| ch.is_whitespace())
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse::<u32>().ok()
}

/// 读小米屏当前音量（0-100）
pub fn mitv_volume_get() -> Result<u32, String> {
    let body = mitv_get("getvolume")?;
    mitv_volume_parse(&body).ok_or_else(|| {
        format!(
            "音量解析失败：{}",
            body.chars().take(80).collect::<String>()
        )
    })
}

/// 小米音量的「目标值」：前端每次拖动只更新这个值，由追赶循环读最新值决定方向。
/// 这样「拖回 20%」会立刻反向降，不会先把旧目标（30%）走完再降 —— 用户实测反馈的痛点。
static MITV_TARGET: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
static MITV_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 把小米屏音量调到 target；本函数会朝**最新目标**逐步逼近后返回最终值。
/// 并发调用时立即返回当前值（不阻塞界面），方向由最后一次设定的目标决定。
pub fn mitv_volume_set(target: u32) -> Result<u32, String> {
    use std::sync::atomic::Ordering;
    MITV_TARGET.store(target.min(100) as i32, Ordering::SeqCst);
    if MITV_BUSY.swap(true, Ordering::SeqCst) {
        return mitv_volume_get();
    }
    let r = (|| -> Result<u32, String> {
        let mut now = mitv_volume_get()?;
        let mut guard = 0;
        loop {
            let t = MITV_TARGET.load(Ordering::SeqCst);
            if t < 0 || t == now as i32 || guard > 150 {
                break;
            }
            let up = t > now as i32;
            // 步进间隔取 220ms：之前用 70ms 高频连打，实测把显示器的 MiTV 服务打到
            // 不应答（ping 通但 6095 端口超时）——1% 一步是固件限制，别把它逼到限流。
            // 失败时等 2 秒重试一次（限流通常是短时的），再失败才向上报错。
            if mitv_get(if up {
                "keyevent&keycode=volumeup"
            } else {
                "keyevent&keycode=volumedown"
            })
            .is_err()
            {
                std::thread::sleep(std::time::Duration::from_millis(2000));
                mitv_get(if up {
                    "keyevent&keycode=volumeup"
                } else {
                    "keyevent&keycode=volumedown"
                })?;
            }
            std::thread::sleep(std::time::Duration::from_millis(220));
            let nv = match mitv_volume_get() {
                Ok(v) => v,
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(1200));
                    match mitv_volume_get() {
                        Ok(v) => v,
                        Err(e) => return Err(e),
                    }
                }
            };
            if nv == now {
                break; // 到顶/到底或没生效
            }
            now = nv;
            guard += 1;
        }
        Ok(now)
    })();
    MITV_BUSY.store(false, Ordering::SeqCst);
    r
}

/// 主屏工作区（已排除任务栏）：(left, top, right, bottom)，物理像素
pub fn primary_work_area() -> Result<(i32, i32, i32, i32), String> {
    let out = ps_core("[SGAudio]::PrimaryWorkArea()")?;
    let t = out.trim();
    let body = t.strip_prefix("WA|").unwrap_or(t);
    let p: Vec<&str> = body.split('|').collect();
    if p.len() >= 4 {
        let n = |i: usize| p[i].trim().parse::<i32>().unwrap_or(0);
        let (l, tp, r, b) = (n(0), n(1), n(2), n(3));
        if r > l && b > tp {
            return Ok((l, tp, r, b));
        }
    }
    Err(format!("工作区解析失败：{}", t.chars().take(60).collect::<String>()))
}

/// 小米屏音量接口是否可用（界面据此决定是否显示这一项）
pub fn mitv_available() -> bool {
    mitv_volume_get().is_ok()
}

// ---------- 一键检测（诊断 + 日志） ----------
// 目的：点一下就把「所有与屏幕有关的信息」扫一遍，逐项判定（正常/警告/问题）并给出处置建议，
// 同时把结果**写进日志**，方便事后回溯（尤其是那种偶发的掉线、颜色不对之类）。

/// 日志目录：%APPDATA%\Screenguard\logs
fn diag_log_dir() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(std::env::var("APPDATA").unwrap_or_default());
    p.push("Screenguard");
    p.push("logs");
    let _ = std::fs::create_dir_all(&p);
    p
}

/// 日志目录路径（界面显示用）
pub fn diag_log_path() -> String {
    diag_log_dir().to_string_lossy().to_string()
}

/// 在文件资源管理器里打开日志目录
pub fn open_log_dir() -> Result<(), String> {
    let d = diag_log_path();
    std::process::Command::new("explorer")
        .arg(&d)
        .spawn()
        .map_err(|e| format!("打开日志目录失败：{}", e))?;
    Ok(())
}

fn portrait_of(res: &str) -> String {
    let p: Vec<&str> = res.split('x').collect();
    if p.len() == 2 {
        if let (Ok(w), Ok(h)) = (p[0].trim().parse::<u32>(), p[1].trim().parse::<u32>()) {
            return if h > w { "竖屏".to_string() } else { "横屏".to_string() };
        }
    }
    "方向未知".to_string()
}

/// 一键检测：扫全部屏幕相关状态 → 逐项判定 + 建议 → 写日志 → 返回报告文本
pub fn diag_scan() -> Result<String, String> {
    let ts = ps("Get-Date -Format \"yyyy-MM-dd HH:mm:ss\"")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let (mut nok, mut nwarn, mut nbad) = (0u32, 0u32, 0u32);
    let mut o = String::new();
    o.push_str("屏幕守护 · 一键检测报告\n");
    o.push_str(&format!("时间：{}\n", ts));
    o.push_str("标记：[正常] 没问题 / [警告] 能用但要留意 / [问题] 需要处理\n");

    // 【1】显示器
    o.push_str("\n【1】显示器\n");
    let ds = get_displays();
    if ds.is_empty() {
        nbad += 1;
        o.push_str("[问题] 一台显示器都没枚举到\n  ↳ 检查线材、供电、KVM 是否切到本机；然后点「显示器健康 → 一键修复」\n");
    }
    for d in &ds {
        let name = if d.name.trim().is_empty() { "(未命名)" } else { d.name.as_str() };
        o.push_str(&format!(
            "[{}] {} · {} · {}Hz · {} · {} · {}\n",
            if d.connected { "正常" } else { "警告" },
            name,
            d.resolution,
            match d.hz { Some(v) => v.to_string(), None => "?".to_string() },
            if d.main { "主屏" } else { "副屏" },
            if d.ddc { "DDC/CI 可用" } else { "DDC/CI 不可用" },
            portrait_of(&d.resolution)
        ));
        o.push_str(&format!(
            "  · 亮度 {} · 音量 {} · 色彩配置 {}\n",
            match d.brightness { Some(v) => format!("{}%", v), None => "读不到".to_string() },
            match d.volume { Some(v) => format!("{}%", v), None => "读不到".to_string() },
            d.color_profile.clone().unwrap_or_else(|| "未读取到".to_string())
        ));
        if d.ddc {
            nok += 1;
        } else {
            nwarn += 1;
            o.push_str("  ↳ 该屏读不到 DDC/CI（常见于 KVM/转接链路不转发）：屏幕自身亮度音量读不到是**链路**问题、非软件故障；亮度用「总亮度(gamma)」兜底，小米这类 Android 屏的音量用「小米音量」滑块\n");
        }
    }

    // 【2】显示链路（近 24 小时）
    o.push_str("\n【2】显示链路（近 24 小时）\n");
    let ev = health_events(1440);
    let evn = ev.lines().filter(|l| !l.trim().is_empty()).count();
    if evn == 0 {
        nok += 1;
        o.push_str("[正常] 没有显示器掉线记录\n");
    } else {
        nwarn += 1;
        o.push_str(&format!(
            "[警告] 有 {} 条掉线相关记录\n  ↳ 常见于 KVM 切换/线材/供电；可用「显示器健康 → 一键修复」强制重协商\n",
            evn
        ));
    }

    // 【3】HDR
    o.push_str("\n【3】HDR\n");
    let hs = hdr_states();
    let mut sup = 0;
    for l in hs.lines().filter(|l| l.starts_with("H|")) {
        let p: Vec<&str> = l.split('|').collect();
        if p.len() >= 4 {
            let s = p[2] == "1";
            if s {
                sup += 1;
            }
            o.push_str(&format!(
                "  {} · {} · 当前 {}\n",
                p[1],
                if s { "支持 HDR" } else { "不支持 HDR" },
                if p[3] == "1" { "开" } else { "关" }
            ));
        }
    }
    if sup == 0 {
        nwarn += 1;
        o.push_str("[警告] 没有检测到支持 HDR 的屏\n");
    } else {
        nok += 1;
        o.push_str("[正常] HDR 状态已读取\n");
    }
    if let Ok(g) = ps("$p=Get-Process AsHDRControl -ErrorAction SilentlyContinue; if($p){\"YES\"}else{\"NO\"}")
    {
        if g.trim() == "YES" {
            nwarn += 1;
            o.push_str("[警告] 厂商 HDR 服务 AsHDRControl 正在运行\n  ↳ HDR 很可能被它接管：程序内切换的 API 会返回成功、但状态不变（本机已实测），要改 HDR 请用「设置 → 系统 → 显示 → HDR」或厂商工具\n");
        }
    }

    // 【4】总亮度（gamma）
    o.push_str("\n【4】总亮度（软件层 gamma）\n");
    let g = gamma_get();
    let mut vals: Vec<String> = Vec::new();
    for l in g.lines().filter(|l| l.starts_with("G|")) {
        let p: Vec<&str> = l.split('|').collect();
        if p.len() >= 3 {
            vals.push(format!("{} = {}%", p[1], p[2]));
        }
    }
    if vals.is_empty() {
        nwarn += 1;
        o.push_str("[警告] 总亮度读取失败（gamma 接口无响应）\n");
    } else {
        o.push_str(&format!("  {}\n", vals.join("　")));
        if vals.iter().all(|v| v.ends_with("= 100%")) {
            nok += 1;
            o.push_str("[正常] 所有屏都在原始曲线（100%）\n");
        } else {
            nwarn += 1;
            o.push_str("[警告] 有屏被软件调过亮度（不是原始曲线）\n  ↳ 点「亮度复位」或「一键恢复默认」可回到原样\n");
        }
    }

    // 【5】音频
    o.push_str("\n【5】音频输出\n");
    match audio_endpoints() {
        Ok(eps) => {
            for e in &eps {
                o.push_str(&format!(
                    "  {} {} · 音量 {}% · {} · {}\n",
                    if e.selected { "▶当前" } else { "　　　" },
                    e.name,
                    e.volume,
                    if e.adjustable { "可调" } else { "固定音量" },
                    if e.is_default { "系统默认设备" } else { "" }
                ));
            }
            match eps.iter().find(|e| e.selected) {
                Some(c) if c.adjustable => {
                    nok += 1;
                    o.push_str(&format!("[正常] 当前控制的「{}」可调音量\n", c.name));
                }
                Some(c) => {
                    nwarn += 1;
                    o.push_str(&format!("[警告] 当前控制的「{}」是**固定音量**端点\n  ↳ Windows 自己的音量条也调不动它。想调这块屏的喇叭用「小米音量」滑块，或在上面的「输出设备」里换一个\n", c.name));
                }
                None => {
                    nwarn += 1;
                    o.push_str("[警告] 没有选中的输出设备\n");
                }
            }
        }
        Err(e) => {
            nbad += 1;
            o.push_str(&format!("[问题] 音频端点枚举失败：{}\n", e));
        }
    }

    // 【6】小米屏（MiTV）
    o.push_str("\n【6】小米显示器（自带 Android 系统）\n");
    if mitv_available() {
        nok += 1;
        o.push_str(&format!(
            "[正常] 显示器音量接口可达，当前音量 {}%\n",
            mitv_volume_get().unwrap_or(0)
        ));
    } else {
        nwarn += 1;
        o.push_str("[警告] 显示器音量接口不可达\n  ↳ 该屏是 Android 系统：确认它与电脑同网段、6095 端口可达；高频率连打会被它限流（通常几分钟自恢复）\n");
    }

    // 【7】电池
    o.push_str("\n【7】电池\n");
    match get_batteries() {
        Ok(bs) if !bs.is_empty() => {
            for b in &bs {
                o.push_str(&format!(
                    "  {} · {}% · {} · {}{}\n",
                    b.name,
                    b.percent,
                    if b.charging { "充电中" } else { "未充电" },
                    b.conn,
                    if b.health > 0 { format!(" · 健康度 {}%", b.health) } else { String::new() }
                ));
                if b.health > 0 && b.health < 70 {
                    nwarn += 1;
                    o.push_str(&format!("  ↳ 健康度 {}% 偏低（若不是保养模式限充，建议关注电池）\n", b.health));
                }
            }
            nok += 1;
        }
        Ok(_) => {
            nwarn += 1;
            o.push_str("[警告] 没有枚举到电池（台式机属正常）\n");
        }
        Err(e) => {
            nwarn += 1;
            o.push_str(&format!("[警告] 电池读取失败：{}\n", e));
        }
    }

    // 【8】基线与一键恢复
    o.push_str("\n【8】基线与一键恢复\n");
    let st = app_state();
    o.push_str(&format!(
        "  基线已采集：{} · HDR {} 屏 · 色彩 {} 屏 · 方向 {} 屏 · 小米音量 {}\n",
        if st.baseline_taken { "是" } else { "否" },
        st.base_hdr.len(),
        st.base_icc.len(),
        st.base_rot.len(),
        st.base_mitv
    ));
    o.push_str(&format!(
        "  退出软件自动还原：{}\n",
        if st.revert_on_exit { "开" } else { "关（可在「一键恢复默认」里打开）" }
    ));
    if !st.baseline_taken {
        nbad += 1;
        o.push_str("[问题] 基线未采集 →「一键恢复默认」不完整\n  ↳ 重启一次本程序即可自动采集\n");
    } else if st.base_rot.is_empty() || st.base_mitv == 0 || st.base_icc.is_empty() {
        nwarn += 1;
        o.push_str("[警告] 基线缺少部分字段（方向 / 小米音量 / 色彩）\n  ↳ 重启一次本程序会补齐\n");
    } else {
        nok += 1;
        o.push_str("[正常] 基线字段完整，「一键恢复默认」可覆盖全部软件层改动\n");
    }

    // 结论
    let total = nok + nwarn + nbad;
    o.push_str(&format!(
        "\n===== 结论 =====\n共 {} 项：{} 正常 / {} 警告 / {} 问题\n",
        total, nok, nwarn, nbad
    ));
    if nbad == 0 && nwarn == 0 {
        o.push_str("全部正常。\n");
    } else if nbad == 0 {
        o.push_str("没有需要修的问题，警告项多为硬件/系统限制（DDC 链路、固定音量端点、厂商 HDR 服务），按建议处置即可。\n");
    } else {
        o.push_str("有问题项，请按上面的「↳」建议处理；处理完可以再点一次检测对比。\n");
    }

    // 写日志：每次检测一个文件 + 追加一行总览，方便回溯
    let fname = format!("diag-{}.log", ts.replace(':', "-").replace(' ', "_"));
    let path = diag_log_dir().join(&fname);
    let mut body = o.clone();
    body.push_str(&format!("日志文件：{}\n", path.to_string_lossy()));
    let _ = std::fs::write(&path, &body);
    let hist = diag_log_dir().join("diag-history.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&hist) {
        use std::io::Write;
        let _ = f.write_all(
            format!("{} | {} 正常 / {} 警告 / {} 问题\n", ts, nok, nwarn, nbad).as_bytes(),
        );
    }
    o.push_str(&format!("\n（已写入日志：{}\\diag-…log）", diag_log_path()));
    Ok(o)
}

/// 一键修复：把检测里「能安全自动处理」的问题逐条修掉，然后自动复查一遍。
/// 原则：只动软件层能确定的事（重协商链路、亮度复位、切到可调音频端点、补齐基线、
/// 还原被铺满的窗口）；硬件/系统限制类的（DDC 链路、厂商 HDR 服务、固定音量端点本身）
/// 不硬修，只如实说明。
pub fn diag_fix() -> Result<String, String> {
    mark_change(); // 修复过程本身会改显示配置，先标记，避免自动修复跟着掺和
    let mut done: Vec<String> = Vec::new();
    let mut skip: Vec<String> = Vec::new();

    // ① 没枚举到显示器 / 有掉线记录 → 强制重协商整条链路
    let evn = health_events(1440)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    let nmon = get_displays().len();
    if nmon == 0 || evn > 0 {
        let lvl = if nmon == 0 { 2 } else { 1 };
        match repair_display(lvl) {
            Ok(o) => done.push(format!(
                "重协商显示链路（{}）：{}",
                if lvl == 2 { "深度" } else { "轻修" },
                o.chars().take(60).collect::<String>()
            )),
            Err(e) => skip.push(format!("重协商显示链路：{}", e)),
        }
    } else {
        done.push("显示链路：近 24 小时无掉线记录，无需重协商".to_string());
    }

    // ② 总亮度不在原始曲线 → 复位
    let g = gamma_get();
    let not_100 = g
        .lines()
        .filter(|l| l.starts_with("G|"))
        .any(|l| !l.trim().ends_with("|100"));
    if not_100 {
        match gamma_set(100) {
            Ok(_) => done.push("总亮度已复位到 100%（原始曲线）".to_string()),
            Err(e) => skip.push(format!("总亮度复位：{}", e)),
        }
    }

    // ③ 当前控制的是「固定音量」端点 → 自动切到一个可调端点
    if let Ok(eps) = audio_endpoints() {
        let bad = eps
            .iter()
            .find(|e| e.selected)
            .map(|e| !e.adjustable)
            .unwrap_or(false);
        if bad {
            let good = eps
                .iter()
                .find(|e| e.adjustable && e.is_default)
                .or_else(|| eps.iter().find(|e| e.adjustable));
            match good {
                Some(t) => match set_audio_device(&t.id) {
                    Ok(_) => done.push(format!("音频控制设备已切到可调的「{}」", t.name)),
                    Err(e) => skip.push(format!("切换音频设备：{}", e)),
                },
                None => skip.push("音频：没有找到可调的输出端点".to_string()),
            }
        }
    }

    // ④ 基线不完整 → 补齐（旋转方向 / 小米音量 / 色彩）
    let st = app_state();
    if !st.baseline_taken || st.base_rot.is_empty() || st.base_mitv == 0 || st.base_icc.is_empty() {
        match capture_baseline() {
            Ok(o) => done.push(format!("恢复基线：{}", o)),
            Err(e) => skip.push(format!("恢复基线：{}", e)),
        }
    }

    // ⑤ 有窗口被铺满还没还原 → 还原
    let _ = restore_foreground();

    // ⑥ 小米屏接口（能修的是「等它自己恢复」，不硬修）
    if !mitv_available() {
        skip.push("小米屏音量接口当前不可达：确认同网段/6095 端口可达；高频连打会被限流，等几分钟再试".to_string());
    }

    let mut o = String::new();
    o.push_str("一键修复结果\n");
    o.push_str(&format!("已处理 {} 项：\n", done.len()));
    for d in &done {
        o.push_str(&format!("  ✓ {}\n", d));
    }
    if !skip.is_empty() {
        o.push_str(&format!("\n无法自动修 / 需你确认 {} 项：\n", skip.len()));
        for s in &skip {
            o.push_str(&format!("  ! {}\n", s));
        }
    }
    o.push_str("\n========== 修复后自动复查 ==========\n");
    match diag_scan() {
        Ok(r) => o.push_str(&r),
        Err(e) => o.push_str(&format!("复查失败：{}\n", e)),
    }
    Ok(o)
}
