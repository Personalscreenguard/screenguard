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

pub fn apply_color_space(_space: &str) -> Result<(), String> {
    // Linux 用 colormgr 或 GNOME 接口，这里留待按桌面环境实现
    Ok(())
}
pub fn match_mac() -> Result<(), String> {
    Ok(())
}
pub fn match_ppi() -> Result<(), String> {
    if which("xrandr") {
        let _ = run_cmd("xrandr", &["--output", "HDMI-1", "--mode", "1920x1080"]);
    }
    Ok(())
}
pub fn rotate_secondary() -> Result<(), String> {
    Ok(())
}

pub fn span_video() -> Result<(), String> {
    if which("vlc") {
        let _ = std::process::Command::new("vlc").spawn();
        Ok(())
    } else {
        Err("未安装 VLC".into())
    }
}
pub fn restore_video() -> Result<(), String> {
    Ok(())
}
