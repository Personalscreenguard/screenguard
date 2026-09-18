use serde::Serialize;
use std::process::Command;

/// 显示器的跨平台结构化描述
#[derive(Serialize, Clone, Debug)]
pub struct DisplayInfo {
    pub id: String,
    pub name: String,
    pub resolution: String,
    pub hz: Option<u32>,
    pub main: bool,
    pub connected: bool,
    pub ddc: bool,
    pub brightness: Option<u32>,
    pub volume: Option<u32>,
    /// 当前色彩配置文件名（ColorSync profile）
    pub color_profile: Option<String>,
    /// 平台特定的显示器句柄（macOS=CGDirectDisplayID；Windows/Linux 用内部 id）
    #[serde(skip_serializing)]
    #[allow(dead_code)] // macOS 用于 DDC 句柄；Windows/Linux 恒为 None，非死代码
    pub ddc_id: Option<u32>,
}

/// 通用命令运行器：返回 stdout（成功）或错误信息。
///
/// 注意：不能只看退出码。PowerShell 的 `-Command` 只要脚本里产生过任何 ErrorRecord
/// （哪怕已被 `-ErrorAction SilentlyContinue` 抑制、stdout 已经输出完整结果）就会返回
/// 退出码 1。实测 get_displays 的脚本正是如此：stdout 是完整的显示器列表，退出码却是 1，
/// 于是整份列表被当成失败丢弃 → 界面显示"未检测到显示器"。
/// 因此以 stdout 为准：有输出就是成功。
pub fn run_cmd(program: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    // CREATE_NO_WINDOW：PowerShell / adb 等子进程不要闪出控制台黑框。
    // 之前漏了这个标志，每次 DDC 读写、蓝牙查询都会在屏幕上闪一个终端窗口。
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("无法启动 {}: {}", program, e))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !stdout.trim().is_empty() {
        return Ok(stdout);
    }
    if out.status.success() {
        Ok(stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr)
            .to_string()
            .trim()
            .to_string())
    }
}

/// 判断某命令是否存在于 PATH（macOS/Linux 用；Windows 走 PowerShell/注册表，无需此函数）
#[cfg(not(target_os = "windows"))]
pub fn which(program: &str) -> bool {
    Command::new("which")
        .arg(program)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 系统默认输出设备信息（CoreAudio 端点）
/// 结构体放共享层：命令签名三平台一致，非 Windows 平台由下方存根返回明确错误
/// （UI 侧音量卡也只在 Windows 显示，存根只是兜底）
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

#[cfg(not(target_os = "windows"))]
pub fn get_system_audio() -> Result<SystemAudio, String> {
    Err("系统音量控制仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn set_system_volume(_v: u32) -> Result<(), String> {
    Err("系统音量控制仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn set_system_mute(_on: bool) -> Result<(), String> {
    Err("系统音量控制仅支持 Windows".to_string())
}

/// 带电池的连接设备（笔记本内电池 / USB·2.4G 无线键鼠 / 蓝牙耳机鼠标等）
/// Windows 经 Battery 设备类枚举；结构体放共享层，非 Windows 由存根兜底
#[derive(Serialize, Clone, Debug)]
pub struct BatteryInfo {
    /// 设备名（注册表友好名，兜底电池栈设备名）
    pub name: String,
    /// 电量百分比 0-100；-1 = 读取不到
    pub percent: i32,
    /// 充电中
    pub charging: bool,
    /// 已接外接电源（满电 / 浮充）
    pub on_ac: bool,
    /// 连接类型：内置 / 蓝牙 / USB/无线 / USB / 其它
    pub conn: String,
}

#[cfg(not(target_os = "windows"))]
pub fn get_batteries() -> Result<Vec<BatteryInfo>, String> {
    // macOS/Linux 的外设电量走 IOKit/upower，后续版本再做；先明确告知
    Err("电池设备监控仅支持 Windows".to_string())
}

// ---------- 屏幕电源控制（方案A）：非 Windows 平台存根 ----------
// macOS/Linux 可分别用 pmset displaysleepnow / xset dpms force off 实现，
// 属后续版本；当前先明确告知，避免 UI 误以为可用。

#[cfg(not(target_os = "windows"))]
pub fn screen_off() -> Result<(), String> {
    Err("屏幕待机控制暂仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn screen_wake() -> Result<(), String> {
    Err("屏幕唤醒暂仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn set_display_power(_display_id: &str, _mode: u32) -> Result<(), String> {
    Err("显示器电源模式控制暂仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn get_display_power(_display_id: &str) -> Result<u32, String> {
    Err("显示器电源模式控制暂仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn display_snapshot() -> Vec<String> {
    Vec::new()
}

// ---------- 显示器健康：KVM 切换 / EDID 重新协商异常检测与修复（非 Windows 存根） ----------

#[cfg(not(target_os = "windows"))]
pub fn health_events(_minutes: u32) -> String {
    String::new()
}

#[cfg(not(target_os = "windows"))]
pub fn mode_detail() -> String {
    String::new()
}

#[cfg(not(target_os = "windows"))]
pub fn repair_display(_level: u32) -> Result<String, String> {
    Err("显示器健康修复暂仅支持 Windows".to_string())
}

// ---------- 方案B：ADB 联网精细控制（可选增强能力） ----------

/// ADB 能力状态：是否找到 adb、已连接哪些设备、给用户的提示
#[derive(serde::Serialize, Clone)]
pub struct AdbStatus {
    pub available: bool,
    pub path: String,
    pub devices: Vec<String>,
    pub tip: String,
}

#[cfg(not(target_os = "windows"))]
pub fn adb_status() -> AdbStatus {
    AdbStatus {
        available: false,
        path: String::new(),
        devices: Vec::new(),
        tip: "ADB 联网控制仅支持 Windows".to_string(),
    }
}

#[cfg(not(target_os = "windows"))]
pub fn adb_connect(_addr: &str) -> Result<String, String> {
    Err("ADB 联网控制仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn adb_power(_serial: &str) -> Result<String, String> {
    Err("ADB 联网控制仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn adb_volume(_serial: &str, _up: bool) -> Result<String, String> {
    Err("ADB 联网控制仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn adb_shell(_serial: &str, _cmd: &str) -> Result<String, String> {
    Err("ADB 联网控制仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn adb_download() -> Result<String, String> {
    Err("ADB 联网控制仅支持 Windows".to_string())
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(target_os = "windows")]
pub use windows::*;
