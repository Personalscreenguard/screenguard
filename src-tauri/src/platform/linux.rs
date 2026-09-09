use super::{run_cmd, which, DisplayInfo};

pub fn platform_name() -> &'static str {
    "linux"
}

/// 解析 `xrandr --query` 得到显示器列表
pub fn get_displays() -> Vec<DisplayInfo> {
    let mut result = Vec::new();
    if let Ok(raw) = run_cmd("xrandr", &["--query"]) {
        for line in raw.lines() {
            let t = line.trim();
            if t.starts_with("connected") {
                // eDP-1 connected 2560x1440+0+0 (main) ...
                let parts: Vec<&str> = t.split_whitespace().collect();
                let id = parts[0].to_string();
                let main = t.contains("primary");
                let res = parts
                    .get(2)
                    .map(|s| s.split('+').next().unwrap_or("").to_string())
                    .unwrap_or_default();
                result.push(DisplayInfo {
                    id,
                    name: format!("显示器 {}", result.len() + 1),
                    resolution: res,
                    hz: None,
                    main,
                    connected: true,
                    ddc: which("ddcutil"),
                    brightness: None,
                    volume: None,
                    color_profile: None,
                    ddc_id: None,
                });
            }
        }
    }
    result
}

/// ddcutil 用 --display N 指定，这里用位置序号
fn index_of(display_id: &str) -> Result<usize, String> {
    get_displays()
        .iter()
        .position(|d| d.id == display_id)
        .map(|i| i + 1)
        .ok_or_else(|| "未找到显示器".into())
}

pub fn set_brightness(display_id: &str, value: u32) -> Result<(), String> {
    if !which("ddcutil") {
        return Err("缺少 ddcutil（apt install ddcutil）".into());
    }
    let idx = index_of(display_id)?;
    run_cmd(
        "ddcutil",
        &[
            "--display",
            &idx.to_string(),
            "setvcp",
            "10",
            &value.clamp(0, 100).to_string(),
        ],
    )
    .map(|_| ())
}

pub fn set_volume(display_id: &str, value: u32) -> Result<(), String> {
    if !which("ddcutil") {
        return Err("缺少 ddcutil（apt install ddcutil）".into());
    }
    let idx = index_of(display_id)?;
    run_cmd(
        "ddcutil",
        &[
            "--display",
            &idx.to_string(),
            "setvcp",
            "62",
            &value.clamp(0, 100).to_string(),
        ],
    )
    .map(|_| ())
}

/// 未实现功能的统一错误（UI 能真实感知，不再假装成功）
fn unsupported(feature: &str) -> Result<(), String> {
    Err(format!("{}：Linux 版暂未实现", feature))
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
    if which("vlc") {
        Err("双屏铺满：Linux 版暂未实现".into())
    } else {
        Err("未安装 VLC".into())
    }
}
pub fn restore_video() -> Result<(), String> {
    unsupported("恢复播放器窗口")
}
