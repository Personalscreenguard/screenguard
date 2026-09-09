use super::{run_cmd, which, DisplayInfo};

pub fn platform_name() -> &'static str {
    "linux"
}

// ===== 开机自启（XDG autostart desktop 文件）=====
const AUTOSTART_DIR: &str = ".config/autostart";
const AUTOSTART_FILE: &str = "screenguard.desktop";

fn autostart_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(home).join(AUTOSTART_DIR).join(AUTOSTART_FILE))
}

fn desktop_entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Screenguard\nComment=跨平台显示器控制与色彩/多屏助手\nExec={}\nX-GNOME-Autostart-enabled=true\n",
        exe
    )
}

pub fn autostart_enabled() -> bool {
    autostart_path().map(|p| p.exists()).unwrap_or(false)
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let path = autostart_path().ok_or("无法定位 ~/.config/autostart")?;
    if enabled {
        let exe = std::env::current_exe()
            .map_err(|e| format!("无法定位程序自身路径：{}", e))?;
        std::fs::create_dir_all(path.parent().ok_or("无效路径")?)
            .map_err(|e| format!("创建 autostart 目录失败：{}", e))?;
        std::fs::write(&path, desktop_entry(&exe.to_string_lossy()))
            .map_err(|e| format!("写入自启配置失败：{}", e))
    } else {
        match std::fs::remove_file(&path) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("移除自启配置失败：{}", e)),
        }
    }
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
