use super::{run_cmd, DisplayInfo};

pub fn platform_name() -> &'static str {
    "windows"
}

const PS: &str = "powershell";

/// 未实现功能的统一错误（UI 能真实感知，不再假装成功）
fn unsupported(feature: &str) -> Result<(), String> {
    Err(format!("{}：Windows 版暂未实现", feature))
}

// C# PInvoke：通过 dxva2 的 Physical Monitor API 读写 DDC VCP
const DDC_CS: &str = r#"
using System;
using System.Runtime.InteropServices;
public class DDC {
  public delegate bool EnumMonProc(IntPtr h, IntPtr hdc, IntPtr r, IntPtr lp);
  [DllImport("user32.dll")] static extern bool EnumDisplayMonitors(IntPtr h, IntPtr r, EnumMonProc cb, IntPtr lp);
  [DllImport("dxva2.dll")] static extern uint GetNumberOfPhysicalMonitorsFromHMONITOR(IntPtr h);
  [DllImport("dxva2.dll")] static extern bool GetPhysicalMonitorsFromHMONITOR(IntPtr h, uint n, PHYS_MON[] a);
  [DllImport("dxva2.dll")] static extern bool SetVCPFeature(IntPtr h, byte v, uint x);
  [DllImport("dxva2.dll")] static extern bool GetVCPFeatureAndVCPFeatureReply(IntPtr h, byte v, ref VCPR r);
  [StructLayout(LayoutKind.Sequential)] struct PHYS_MON { public IntPtr h; [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string n; }
  [StructLayout(LayoutKind.Sequential)] struct VCPR { public uint ver, cur, max, min; }
  static System.Collections.Generic.List<IntPtr> Mons() {
    var l = new System.Collections.Generic.List<IntPtr>();
    EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, (h,hdc,r,lp)=>{
      uint n = GetNumberOfPhysicalMonitorsFromHMONITOR(h);
      var a = new PHYS_MON[n];
      GetPhysicalMonitorsFromHMONITOR(h, n, a);
      foreach (var m in a) l.Add(m.h);
      return true;
    }, IntPtr.Zero);
    return l;
  }
  public static uint Read(byte v) {
    foreach (var h in Mons()) { var r = new VCPR(); if (GetVCPFeatureAndVCPFeatureReply(h, v, ref r)) return r.cur; }
    return 0;
  }
  public static void Write(byte v, uint x) { foreach (var h in Mons()) SetVCPFeature(h, v, x); }
}
"#;

/// 运行一段 PowerShell，先注入 DDC 类再执行 op
/// （C# 源码用单引号包裹传入 Add-Type，规避 here-string 的换行限制；
///   注意 DDC_CS 内不含单引号，仅双引号，可安全嵌入单引号串）
fn wddc(op: &str) -> Result<String, String> {
    let script = format!("Add-Type -TypeDefinition '{}'; {}", DDC_CS, op);
    run_cmd(
        PS,
        &[
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ],
    )
}

/// 枚举显示器（WmiMonitorID）
pub fn get_displays() -> Vec<DisplayInfo> {
    let mut result = Vec::new();
    let cmd = "Get-CimInstance -Namespace root/wmi -ClassName WmiMonitorID | ForEach-Object { $n = ($_.UserFriendlyName | Where-Object {$_ -ne 0} | ForEach-Object {[char]$_}) -join ''; Write-Output ($_.InstanceName + '|' + $n) }";
    if let Ok(raw) = run_cmd(PS, &["-NoProfile", "-Command", cmd]) {
        for line in raw.lines().filter(|l| l.contains('|')) {
            let mut parts = line.splitn(2, '|');
            let id = parts.next().unwrap_or("").trim().to_string();
            let name = parts.next().unwrap_or("显示器").trim().to_string();
            if id.is_empty() {
                continue;
            }
            result.push(DisplayInfo {
                id: id.clone(),
                name: if name.is_empty() {
                    format!("显示器 {}", result.len() + 1)
                } else {
                    name
                },
                resolution: String::new(),
                hz: None,
                main: false,
                connected: true,
                ddc: true,
                brightness: None,
                volume: None,
                color_profile: None,
                ddc_id: None,
            });
        }
    }
    result
}

pub fn set_brightness(_display_id: &str, value: u32) -> Result<(), String> {
    let v = value.clamp(0, 100);
    wddc(&format!("[DDC]::Write(0x10,{})", v)).map(|_| ())
}

pub fn set_volume(_display_id: &str, value: u32) -> Result<(), String> {
    let v = value.clamp(0, 100);
    wddc(&format!("[DDC]::Write(0x62,{})", v)).map(|_| ())
}

pub fn apply_color_space(_space: &str) -> Result<(), String> {
    unsupported("色彩空间同步")
}
pub fn match_mac() -> Result<(), String> {
    unsupported("对齐 Mac 内建屏")
}
pub fn match_ppi() -> Result<(), String> {
    unsupported("窗口跨屏等大")
}
pub fn rotate_secondary() -> Result<(), String> {
    unsupported("副屏横竖屏切换")
}
pub fn restore_secondary() -> Result<(), String> {
    unsupported("恢复副屏竖屏")
}
pub fn span_video() -> Result<(), String> {
    unsupported("双屏铺满")
}
pub fn restore_video() -> Result<(), String> {
    unsupported("恢复播放器窗口")
}
