//! Windows 平台实现：全部通过 PowerShell + 内嵌 C# (Win32 P/Invoke) 落地，
//! 延续项目「零额外 Rust 依赖」的风格。每个命令独立 powershell 进程，
//! C# 代码用单引号包裹传入 Add-Type（C# 内不含单引号字符）。

use super::{run_cmd, DisplayInfo};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub fn platform_name() -> &'static str {
    "windows"
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
        run_cmd(
            "reg",
            &["add", RUN_KEY, "/v", RUN_NAME, "/t", "REG_SZ", "/d", &exe, "/f"],
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

[StructLayout(LayoutKind.Explicit)]
public struct SGDEVMODE {
  [FieldOffset(64)] public ushort dmSpecVersion;
  [FieldOffset(66)] public ushort dmDriverVersion;
  [FieldOffset(68)] public ushort dmSize;
  [FieldOffset(70)] public ushort dmDriverExtra;
  [FieldOffset(72)] public uint dmFields;
  [FieldOffset(76)] public short dmOrientation;
  [FieldOffset(84)] public uint dmDisplayOrientation;
  [FieldOffset(88)] public uint dmDisplayFixedOutput;
  [FieldOffset(92)] public short dmColor;
  [FieldOffset(166)] public ushort dmLogPixels;
  [FieldOffset(168)] public uint dmBitsPerPel;
  [FieldOffset(172)] public uint dmPelsWidth;
  [FieldOffset(176)] public uint dmPelsHeight;
  [FieldOffset(180)] public uint dmDisplayFlags;
  [FieldOffset(184)] public uint dmDisplayFrequency;
  [FieldOffset(188)] public uint dmICMMethod;
  [FieldOffset(204)] public uint dmReserved1;
  [FieldOffset(212)] public uint dmPanningWidth;
  [FieldOffset(216)] public uint dmPanningHeight;
}

[StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
public struct SGDISPLAY_DEVICE {
  public int cb;
  [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string DeviceName;
  [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceString;
  // 关键：实测本机(2026, Win11 24H2+) EnumDisplayDevicesW 实际布局在 DeviceString
  // 后多 4 字节(疑似新 SDK 隐藏字段)，缺此 pad 会导致 DeviceID/DeviceKey 整体
  // 错位 4 字节，读到 \u0003 之类垃圾 → uid 全坏、按屏操作无法定位。勿删！
  public int _pad;
  [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceID;
  [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceKey;
  public int StateFlags;
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

public class SGCore {
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool EnumDisplayDevicesW(string dev, uint i, ref SGDISPLAY_DEVICE d, uint f);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool EnumDisplaySettingsW(string dev, int mode, IntPtr dm);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int ChangeDisplaySettingsExW(string dev, IntPtr dm, IntPtr nul, uint flags, IntPtr p);
  [DllImport("user32.dll")] static extern int GetSystemMetrics(int idx);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool GetMonitorInfo(IntPtr h, ref SGMONITORINFO mi);
  [DllImport("user32.dll")] static extern bool EnumDisplayMonitors(IntPtr h, IntPtr rc, EnumMonProc cb, IntPtr lp);
  delegate bool EnumMonProc(IntPtr h, IntPtr hdc, IntPtr rc, IntPtr lp);

  [DllImport("dxva2.dll")] static extern bool GetNumberOfPhysicalMonitorsFromHMONITOR(IntPtr h, out uint n);
  [DllImport("dxva2.dll")] static extern bool GetPhysicalMonitorsFromHMONITOR(IntPtr h, uint n, SGPHYS_MON[] arr);
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

  const int GWL_STYLE = -16;
  const uint WS_CAPTION = 0x00C00000, WS_THICKFRAME = 0x00040000, WS_POPUP = 0x80000000;
  const uint SWP_NOZORDER = 0x0004, SWP_NOACTIVATE = 0x0010, SWP_FRAMECHANGED = 0x0020, SWP_SHOWWINDOW = 0x0040;
  const uint DM_DISPLAYORIENTATION = 0x00000080;
  const uint DM_PELSWIDTH = 0x00080000, DM_PELSHEIGHT = 0x00100000, DM_DISPLAYFREQUENCY = 0x00400000;
  const uint CDS_UPDATEREGISTRY = 0x0001, CDS_GLOBAL = 0x0008;
  const uint SPI_GETWORKAREA = 0x0030;

  static IntPtr AllocDevMode() { return Marshal.AllocHGlobal(220); }

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
            var dm = Marshal.PtrToStructure<SGDEVMODE>(b);
            string uid = "";
            uint j = 0;
            while (true) {
              var m = new SGDISPLAY_DEVICE();
              m.cb = Marshal.SizeOf(typeof(SGDISPLAY_DEVICE));
              if (!EnumDisplayDevicesW(dev, j, ref m, 0)) break;
              uid = m.DeviceID;
              j++;
            }
            int main = (mi.dwFlags & 1) != 0 ? 1 : 0;
            sb.Append("D|").Append(dev).Append("|").Append(uid).Append("|")
              .Append(dm.dmPelsWidth).Append("|").Append(dm.dmPelsHeight).Append("|")
              .Append(dm.dmDisplayFrequency).Append("|").Append(main).Append("|")
              .Append(dm.dmDisplayOrientation).Append("\n");
          }
        } finally { Marshal.FreeHGlobal(b); }
      }
      return true;
    }, IntPtr.Zero);
    return sb.ToString();
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
  public static string Rotate(string dev, uint newOri) {
    var b = AllocDevMode();
    try {
      if (!EnumDisplaySettingsW(dev, -1, b)) return "ERR:读取当前显示模式失败";
      var dm = Marshal.PtrToStructure<SGDEVMODE>(b);
      uint curOri = dm.dmDisplayOrientation;
      bool curP = (curOri == 1 || curOri == 3);
      bool newP = (newOri == 1 || newOri == 3);
      uint w = dm.dmPelsWidth, h = dm.dmPelsHeight;
      if (curP != newP) { uint t = w; w = h; h = t; }
      dm.dmDisplayOrientation = newOri;
      dm.dmPelsWidth = w;
      dm.dmPelsHeight = h;
      dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_DISPLAYORIENTATION;
      dm.dmSize = 220;
      Marshal.StructureToPtr(dm, b, false);
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
    let script = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\nAdd-Type -TypeDefinition '{cs}';\n[SGCore]::ListDisplays()",
        cs = CORE_CS
    );
    let raw = ps(&script)?;
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
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\nAdd-Type -TypeDefinition '{cs}';\n$lines = [SGCore]::ListDisplays();\n$lines | ForEach-Object {{ Write-Output $_ }};\n$devs = @();\nforeach ($l in $lines) {{ $p = $l -split '\\|'; if ($p[0] -eq 'D') {{ $devs += $p[1] }} }};\nGet-CimInstance -Namespace root/wmi -ClassName WmiMonitorID -ErrorAction SilentlyContinue | ForEach-Object {{\n  $nm = (($_.UserFriendlyName | Where-Object {{ [int]$_ -ne 0 }} | ForEach-Object {{ [char][int]$_ }}) -join '');\n  $inst = $_.InstanceName -replace '_\\d+$', '';\n  if ($nm -and $nm.Trim()) {{ Write-Output ('N|' + $inst + '|' + $nm) }}\n}};\nforeach ($dev in $devs) {{ Write-Output ('P|' + $dev + '|' + [SGCore]::DDCProbe($dev)) }}",
        cs = CORE_CS
    );
    let raw = ps(&script).unwrap_or_default();
    let mut rows: Vec<(String, String, u32, u32, u32, bool, u32)> = Vec::new(); // dev,uid,w,h,hz,main,ori
    let mut names: HashMap<String, String> = HashMap::new();
    let mut ddc: HashMap<String, bool> = HashMap::new();
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
            "P" if p.len() >= 3 => {
                ddc.insert(p[1].to_string(), p[2].trim() == "1");
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
    let mut result: Vec<DisplayInfo> = Vec::new();
    for (dev, uid, w, h, hz, main, ori) in rows {
        let name = names
            .get(&short_id(&uid))
            .cloned()
            .filter(|n| !n.is_empty())
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
            brightness: None,
            volume: None,
            color_profile: None,
            ddc_id: None,
        });
    }
    result
}

// ---------- DDC：亮度 / 音量（按显示器精确控制） ----------
fn vcp_op(display_id: &str, code: u8, value: Option<u32>) -> Result<(), String> {
    let dev = resolve_dev(display_id)?;
    let script = match value {
        Some(v) => format!(
            "Add-Type -TypeDefinition '{cs}';\n[SGCore]::DDCWrite('{dev}',[byte]{code},[uint32]{v})",
            cs = CORE_CS,
            dev = dev,
            code = code,
            v = v
        ),
        None => format!(
            "Add-Type -TypeDefinition '{cs}';\n[SGCore]::DDCRead('{dev}',[byte]{code})",
            cs = CORE_CS,
            dev = dev,
            code = code
        ),
    };
    let out = ps(&script)?;
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
    vcp_op(display_id, 0x10, Some(v))
}

pub fn set_volume(display_id: &str, value: u32) -> Result<(), String> {
    let v = value.clamp(0, 100);
    vcp_op(display_id, 0x62, Some(v))
}

// ---------- 色彩同步 ----------
const COLOR_DIR: &str = "screenguard_icc";

/// 定位目标 ICC：sRGB 用系统内置；P3 / AdobeRGB 需要在用户目录存在（可由用户放置/下载）
fn icc_path(space: &str) -> Result<String, String> {
    match space.to_lowercase().as_str() {
        "srgb" => Ok("C:\\Windows\\System32\\spool\\drivers\\color\\sRGB Color Space Profile.icm".to_string()),
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
                    "缺少色彩配置文件：请将 {} 放到 {}（Windows 无内置 Display P3/AdobeRGB 配置）",
                    file, dir
                ))
            }
        }
        _ => Err("未知色彩空间".to_string()),
    }
}

/// Windows 端「色彩同步」：把 ICC 配置文件关联到所有显示设备。
/// 关联本身无副作用、可随时改回，失败会真实报错。
pub fn apply_color_space(space: &str) -> Result<(), String> {
    let profile = icc_path(space)?;
    // 依次取各显示设备的 device path（\\?\DISPLAY#...#{e6f07b5f-...}）并关联
    let script = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\n\
         $prof = '{profile}';\n\
         $mon = Get-PnpDevice -Class Monitor -Status OK -ErrorAction SilentlyContinue;\n\
         $ok = 0; $errs = @();\n\
         foreach ($m in $mon) {{\n\
           try {{\n\
             $devPath = '\\\\?\\DISPLAY#' + $m.InstanceId + '#{{e6f07b5f-ee97-4a90-b076-33f57bf4ba84}}';\n\
             Add-Type -TypeDefinition '\n\
             using System; using System.Runtime.InteropServices;\n\
             public class SGWCS {{\n\
               [DllImport(\"mscms.dll\", CharSet = CharSet.Unicode)] public static extern bool WcsAssociateColorProfileWithDevice(IntPtr h, string prof, string dev);\n\
               [DllImport(\"mscms.dll\", CharSet = CharSet.Unicode)] public static extern bool WcsDisassociateColorProfileFromDevice(IntPtr h, string prof, string dev);\n\
               [DllImport(\"mscms.dll\", CharSet = CharSet.Unicode)] public static extern uint InstallColorProfileW(IntPtr h, string prof);\n\
             }}';\n\
             [SGWCS]::InstallColorProfileW([IntPtr]::Zero, $prof) | Out-Null;\n\
             if ([SGWCS]::WcsAssociateColorProfileWithDevice([IntPtr]::Zero, $prof, $devPath)) {{ $ok++ }} else {{ $errs += $m.FriendlyName }}\n\
           }} catch {{ $errs += $m.FriendlyName }}\n\
         }};\n\
         if ($ok -gt 0) {{ Write-Output ('OK:' + $ok) }} else {{ Write-Output ('ERR:' + ($errs -join ',')) }}",
        profile = profile
    );
    let out = ps(&script)?;
    let t = out.trim();
    if t.starts_with("OK") {
        Ok(())
    } else {
        Err(format!("色彩关联失败：{}", t))
    }
}

/// 对齐 Mac 内建屏 = 将外接屏对齐 Display P3（Mac 内建屏为 P3）
pub fn match_mac() -> Result<(), String> {
    apply_color_space("p3")
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
    let script = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\nAdd-Type -TypeDefinition '{cs}';\n[SGCore]::ListDisplays()",
        cs = CORE_CS
    );
    let raw = ps(&script)?;
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
    let script = format!(
        "Add-Type -TypeDefinition '{cs}';\n[SGCore]::Rotate('{dev}',[uint32]{target})",
        cs = CORE_CS,
        dev = dev,
        target = target
    );
    let out = ps(&script)?;
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
    let script = format!(
        "Add-Type -TypeDefinition '{cs}';\n[SGCore]::Rotate('{dev}',[uint32]3)",
        cs = CORE_CS,
        dev = dev
    );
    let out = ps(&script)?;
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
    // 2) 启动 PotPlayer
    let candidates = [
        "C:\\Program Files\\DAUM\\PotPlayer\\PotPlayerMini64.exe",
        "C:\\Program Files (x86)\\DAUM\\PotPlayer\\PotPlayerMini.exe",
        "C:\\Program Files\\VideoLAN\\VLC\\vlc.exe",
        "C:\\Program Files (x86)\\VideoLAN\\VLC\\vlc.exe",
    ];
    for path in candidates.iter() {
        if std::path::Path::new(path).exists() {
            let _ = std::process::Command::new(path).spawn();
            std::thread::sleep(std::time::Duration::from_millis(2500));
            if let Ok(out) = ps(probe) {
                let t = out.trim();
                if !t.is_empty() {
                    if let Ok(h) = t.parse::<i64>() {
                        return Ok(h);
                    }
                }
            }
            break;
        }
    }
    Err("未找到播放器：请先打开 PotPlayer 或 VLC（双屏铺满依赖播放器窗口）".to_string())
}

/// 双屏铺满：播放器窗口拉伸到所有屏幕的包围盒
pub fn span_video() -> Result<(), String> {
    let hwnd = player_hwnd()?;
    let script = format!(
        "Add-Type -TypeDefinition '{cs}';\n[SGCore]::SpanVideo([IntPtr]{hwnd}, '{sf}')",
        cs = CORE_CS,
        hwnd = hwnd,
        sf = state_file()
    );
    let out = ps(&script)?;
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
    let script = format!(
        "Add-Type -TypeDefinition '{cs}';\n[SGCore]::RestoreVideo([IntPtr]{hwnd}, '{sf}')",
        cs = CORE_CS,
        hwnd = hwnd,
        sf = state_file()
    );
    let out = ps(&script)?;
    let t = out.trim();
    if t == "OK" {
        Ok(())
    } else if t.starts_with("ERR:") {
        Err(t[4..].to_string())
    } else {
        Err(t.to_string())
    }
}
