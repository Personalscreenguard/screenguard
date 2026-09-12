mod platform;

use platform::DisplayInfo;

/// 返回当前平台名
#[tauri::command]
fn get_platform() -> &'static str {
    platform::platform_name()
}

/// 返回 App 版本号（与 Cargo.toml / tauri.conf.json 一致）
#[tauri::command]
fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// 开机自启是否已启用
#[tauri::command]
fn get_autostart() -> bool {
    platform::autostart_enabled()
}

/// 设置开机自启
#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
    platform::set_autostart(enabled)
}

#[tauri::command]
fn get_displays() -> Vec<DisplayInfo> {
    platform::get_displays()
}

#[tauri::command]
fn set_brightness(display_id: String, value: u32) -> Result<(), String> {
    platform::set_brightness(&display_id, value)
}

#[tauri::command]
fn set_volume(display_id: String, value: u32) -> Result<(), String> {
    platform::set_volume(&display_id, value)
}

/// 系统默认输出设备信息（名称 / 音量 / 静音 / 是否可调 / 形态因子）
#[tauri::command]
fn get_system_audio() -> Result<platform::SystemAudio, String> {
    platform::get_system_audio()
}

/// 设置系统默认输出设备音量 0-100（托盘滑块同源；笔记本喇叭/耳机/HDMI 音频都走这里）
#[tauri::command]
fn set_system_volume(value: u32) -> Result<(), String> {
    platform::set_system_volume(value)
}

/// 系统静音开关
#[tauri::command]
fn set_system_mute(on: bool) -> Result<(), String> {
    platform::set_system_mute(on)
}

#[tauri::command]
fn apply_color_space(space: String) -> Result<(), String> {
    platform::apply_color_space(&space)
}

#[tauri::command]
fn match_mac() -> Result<(), String> {
    platform::match_mac()
}

#[tauri::command]
fn match_ppi() -> Result<(), String> {
    platform::match_ppi()
}

#[tauri::command]
fn rotate_secondary() -> Result<(), String> {
    platform::rotate_secondary()
}

/// 恢复副屏为竖屏
#[tauri::command]
fn restore_secondary() -> Result<(), String> {
    platform::restore_secondary()
}

#[tauri::command]
fn span_video() -> Result<(), String> {
    platform::span_video()
}

#[tauri::command]
fn restore_video() -> Result<(), String> {
    platform::restore_video()
}

/// 显示并聚焦主窗口（命令、托盘菜单共用）
fn show_main_window(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// 菜单栏面板：打开主窗口
#[tauri::command]
fn open_main(app: tauri::AppHandle) -> Result<(), String> {
    show_main_window(&app);
    Ok(())
}

/// 菜单栏面板：退出
#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// 菜单栏面板：隐藏浮窗（不退出应用；面板的 ✕ 按钮与 Esc 键调用）
#[tauri::command]
fn hide_panel(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("panel") {
        w.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            use tauri::menu::{Menu, MenuItem};
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
            use tauri::Manager;

            // 单实例自检：若已有更早启动的实例在跑，本实例直接退出，防双托盘
            // （macOS 用于避免 launchd bootstrap 双开；Windows 用于避免双击图标开出多个托盘）
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            if platform::another_instance_running() {
                std::process::exit(0);
            }

            // 右键菜单
            let open_main = MenuItem::with_id(app, "open_main", "打开主窗口", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_main, &quit])?;

            // 显示/隐藏面板浮窗（右上角、菜单栏下方）
            let toggle_panel = |app_handle: &tauri::AppHandle| {
                if let Some(pw) = app_handle.get_webview_window("panel") {
                    if pw.is_visible().unwrap_or(false) {
                        let _ = pw.hide();
                    } else {
                        if let Ok(Some(mon)) = pw.primary_monitor() {
                            let scale = mon.scale_factor();
                            let x = (mon.size().width as f64 - 360.0 * scale - 24.0 * scale) as i32;
                            let y = (28.0 * scale) as i32;
                            let _ = pw.set_position(tauri::PhysicalPosition::new(x, y));
                        }
                        let _ = pw.show();
                        let _ = pw.set_focus();
                    }
                }
            };

            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().expect("no icon").clone())
                .tooltip("屏幕守护")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app_handle, event| match event.id.as_ref() {
                    "open_main" => show_main_window(app_handle),
                    "quit" => app_handle.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(move |tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_panel(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // 主窗口点 X = 隐藏到托盘（程序常驻，托盘左键呼出面板）
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_platform,
            get_version,
            get_autostart,
            set_autostart,
            get_displays,
            set_brightness,
            set_volume,
            get_system_audio,
            set_system_volume,
            set_system_mute,
            apply_color_space,
            match_mac,
            match_ppi,
            rotate_secondary,
            restore_secondary,
            span_video,
            restore_video,
            open_main,
            quit_app,
            hide_panel
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
