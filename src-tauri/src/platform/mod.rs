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
