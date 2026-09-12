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

/// 通用命令运行器：返回 stdout（成功）或错误信息
pub fn run_cmd(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("无法启动 {}: {}", program, e))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
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
