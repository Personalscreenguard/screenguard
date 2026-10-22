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
    /// 平台特定的显示器句柄（macOS=CGDirectDisplayID；Windows 用内部 id）
    #[serde(skip_serializing)]
    #[allow(dead_code)] // macOS 用于 DDC 句柄；Windows 恒为 None，非死代码
    pub ddc_id: Option<u32>,
}

/// Operating-system sleep state for one display; this does not claim physical panel power.
#[derive(Serialize, Clone, Debug)]
pub struct DisplaySleepState {
    pub id: String,
    pub asleep: bool,
}

/// macOS 当前旋转方向可用的显示模式。
#[derive(Serialize, Clone, Debug)]
pub struct DisplayMode {
    pub mode: u32,
    pub resolution: String,
    pub hz: u32,
    pub color_depth: u32,
    pub scaled: bool,
    pub current: bool,
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

/// Open one of the application's fixed public pages in the system browser.
#[cfg(target_os = "macos")]
pub fn open_external_page(page: &str) -> Result<(), String> {
    let url = match page {
        "releases" => "https://github.com/Personalscreenguard/screenguard/releases/latest",
        "privacy" => "https://github.com/Personalscreenguard/screenguard/blob/main/PRIVACY.md",
        _ => return Err("不支持打开此外部页面".into()),
    };
    Command::new("/usr/bin/open").arg(url).spawn()
        .map(|_| ()).map_err(|e| format!("无法打开系统浏览器：{}", e))
}

#[cfg(target_os = "windows")]
pub fn open_external_page(page: &str) -> Result<(), String> {
    let url = match page {
        "releases" => "https://github.com/Personalscreenguard/screenguard/releases/latest",
        "privacy" => "https://github.com/Personalscreenguard/screenguard/blob/main/PRIVACY.md",
        _ => return Err("不支持打开此外部页面".into()),
    };
    Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler", url]).spawn()
        .map(|_| ()).map_err(|e| format!("无法打开系统浏览器：{}", e))
}

/// 判断某命令是否存在于 PATH（macOS 用；Windows 走 PowerShell/注册表，无需此函数）
#[cfg(not(target_os = "windows"))]
pub fn which(program: &str) -> bool {
    Command::new("which")
        .arg(program)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 系统默认输出设备信息（Windows CoreAudio 端点 / macOS 系统默认输出）
/// 结构体放共享层：命令签名两平台一致。
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

/// 带电池的连接设备（笔记本内电池 / USB·2.4G 无线键鼠 / 蓝牙耳机鼠标等）
/// Windows 枚举内置/外设电池；macOS 返回内置电池；结构体放共享层。
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
    /// 健康度（满充容量/设计容量 ×100）；-1 = 该设备不提供
    pub health: i32,
    /// 当前容量（mWh；相对模式的 HID 外设为 0）
    pub cap_mwh: u32,
    /// 满充容量（mWh；相对模式的 HID 外设为 0）
    pub full_mwh: u32,
}

// ---------- 软件层总亮度 / HDR / 前台窗口铺满 / 全局快捷键（非 Windows 存根） ----------

#[cfg(not(target_os = "windows"))]
pub fn gamma_get() -> String {
    String::new()
}

#[cfg(not(target_os = "windows"))]
pub fn gamma_set(_pct: u32) -> Result<String, String> {
    Err("总亮度控制仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn hdr_states() -> String {
    String::new()
}

#[cfg(not(target_os = "windows"))]
pub fn hdr_set(_on: bool) -> Result<String, String> {
    Err("HDR 同步仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn match_dpi_apply() -> Result<String, String> {
    Err("跨屏缩放对齐仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn match_dpi_restore() -> Result<String, String> {
    Err("跨屏缩放对齐仅支持 Windows".to_string())
}

/// 音频输出端点（Windows 可枚举/切换；macOS 暴露系统默认端点）
#[derive(Serialize, Clone, Debug)]
pub struct AudioEndpoint {
    pub id: String,
    pub name: String,
    pub volume: u32,
    pub mute: bool,
    pub adjustable: bool,
    pub form_factor: u32,
    /// 是否是当前用户选定要控制的那一个
    pub selected: bool,
    /// 是否是 Windows 当前的默认输出设备（用户没明确选时优先控制它）
    pub is_default: bool,
}

#[cfg(not(target_os = "windows"))]
pub fn hdr_set_dev(_dev: &str, _on: bool) -> Result<String, String> {
    Err("HDR 控制仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn mitv_volume_get() -> Result<u32, String> {
    Err("小米显示器音量控制仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn mitv_volume_set(_target: u32) -> Result<u32, String> {
    Err("小米显示器音量控制仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn mitv_available() -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
pub fn primary_work_area() -> Result<(i32, i32, i32, i32), String> {
    Err("仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn standby_state() -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
pub fn diag_scan() -> Result<String, String> {
    Err("一键检测仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn diag_fix() -> Result<String, String> {
    Err("一键修复仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn recently_changed(_secs: u64) -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
pub fn open_log_dir() -> Result<(), String> {
    Err("仅支持 Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn diag_log_path() -> String {
    String::new()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn align_report() -> Result<String, String> {
    Err("当前平台暂不支持双屏对齐体检".to_string())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn align_apply() -> Result<String, String> {
    Err("当前平台暂不支持应用双屏对齐组合".to_string())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn align_restore_modes() -> Result<String, String> {
    Err("当前平台暂不支持还原双屏模式".to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn get_display_modes(_display_id: &str) -> Result<Vec<DisplayMode>, String> {
    Err("当前平台暂不支持读取显示模式".to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn set_display_mode(_display_id: &str, _mode: u32) -> Result<String, String> {
    Err("当前平台暂不支持切换显示模式".to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn display_action(
    _action: &str,
    _display_id: &str,
    _target_id: Option<&str>,
    _value: Option<&str>,
) -> Result<String, String> {
    Err("此显示控制目前仅支持 macOS".to_string())
}

// ---------- 屏幕电源控制 ----------
#[cfg(not(target_os = "macos"))]
pub fn display_sleep_states() -> Vec<DisplaySleepState> {
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

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(target_os = "windows")]
pub use windows::*;
