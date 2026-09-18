mod platform;

use platform::DisplayInfo;

// ============================================================================
// 关于「为什么这些命令都是 async + spawn_blocking」
// ----------------------------------------------------------------------------
// Tauri 的**同步**命令是跑在**主线程**上的。而本应用几乎每个平台调用都要起一个
// PowerShell / adb 子进程：一次 DDC 读写约 0.5 秒，蓝牙电池查询更久。同步写法的后果
// 就是——拖一下亮度滑块，界面卡半秒；点一下按钮，界面卡半秒；联动每 2.5 秒轮询一次，
// 界面每隔 2.5 秒顿一下。
//
// 统一改成 async + tauri::async_runtime::spawn_blocking 之后，阻塞工作落到专用线程池，
// 主线程（UI）不再被占用。**新增平台命令时请沿用这个模式**，除非它真的只是瞬时的内存操作。
// 例外：窗口/托盘相关的命令（open_main / quit_app / hide_panel）必须留在主线程，保持同步。
// ============================================================================

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
async fn get_autostart() -> bool {
    tauri::async_runtime::spawn_blocking(platform::autostart_enabled)
        .await
        .unwrap_or(false)
}

/// 设置开机自启
#[tauri::command]
async fn set_autostart(enabled: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || platform::set_autostart(enabled))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

#[tauri::command]
async fn get_displays() -> Vec<DisplayInfo> {
    tauri::async_runtime::spawn_blocking(platform::get_displays)
        .await
        .unwrap_or_default()
}

#[tauri::command]
async fn set_brightness(display_id: String, value: u32) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || platform::set_brightness(&display_id, value))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

#[tauri::command]
async fn set_volume(display_id: String, value: u32) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || platform::set_volume(&display_id, value))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 系统默认输出设备信息（名称 / 音量 / 静音 / 是否可调 / 形态因子）
#[tauri::command]
async fn get_system_audio() -> Result<platform::SystemAudio, String> {
    tauri::async_runtime::spawn_blocking(platform::get_system_audio)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 设置系统默认输出设备音量 0-100（托盘滑块同源；笔记本喇叭/耳机/HDMI 音频都走这里）
#[tauri::command]
async fn set_system_volume(value: u32) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || platform::set_system_volume(value))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 系统静音开关
#[tauri::command]
async fn set_system_mute(on: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || platform::set_system_mute(on))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 电池设备列表（带电池的连接设备：笔记本内电池 / USB·无线键鼠 / 蓝牙）
#[tauri::command]
async fn get_batteries() -> Result<Vec<platform::BatteryInfo>, String> {
    tauri::async_runtime::spawn_blocking(platform::get_batteries)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

#[tauri::command]
async fn apply_color_space(space: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || platform::apply_color_space(&space))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

#[tauri::command]
async fn match_mac() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(platform::match_mac)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

#[tauri::command]
async fn match_ppi() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(platform::match_ppi)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

#[tauri::command]
async fn rotate_secondary() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(platform::rotate_secondary)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 恢复副屏为竖屏
#[tauri::command]
async fn restore_secondary() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(platform::restore_secondary)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

#[tauri::command]
async fn span_video() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(platform::span_video)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

#[tauri::command]
async fn restore_video() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(platform::restore_video)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

// ---------- 屏幕电源控制（方案A：纯本地） ----------

/// 系统级关闭所有屏幕（一次黑全部屏，含不支持 DDC 的显示器）
#[tauri::command]
async fn screen_off() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(platform::screen_off)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 唤醒屏幕（合成输入事件，解除待机）
#[tauri::command]
async fn screen_wake() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(platform::screen_wake)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 单台显示器电源模式（DDC 0xD6）：1=开 2=待机 4=软关
#[tauri::command]
async fn set_display_power(display_id: String, mode: u32) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || platform::set_display_power(&display_id, mode))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 读单台显示器电源模式（1=开 2=待机 4=软关 5=硬关）
#[tauri::command]
async fn get_display_power(display_id: String) -> Result<u32, String> {
    tauri::async_runtime::spawn_blocking(move || platform::get_display_power(&display_id))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 显示器在线快照（联动检测轮询用，轻量）
#[tauri::command]
async fn display_snapshot() -> Vec<String> {
    tauri::async_runtime::spawn_blocking(platform::display_snapshot)
        .await
        .unwrap_or_default()
}

// ---------- 显示器健康：KVM/EDID 异常检测与一键修复（无需管理员权限） ----------

/// 读取最近的显示链路异常事件（KVM 切换、显示器突然离线、EDID 协商失败等）
#[tauri::command]
async fn health_events(minutes: Option<u32>) -> String {
    let m = minutes.unwrap_or(1440);
    tauri::async_runtime::spawn_blocking(move || platform::health_events(m))
        .await
        .unwrap_or_default()
}

/// 每台在用屏幕的当前模式（分辨率/刷新率/方向）
#[tauri::command]
async fn mode_detail() -> String {
    tauri::async_runtime::spawn_blocking(platform::mode_detail)
        .await
        .unwrap_or_default()
}

/// 一键修复显示异常：level 1 = 强制重新协商（轻）；level 2 = 含链路重握手（深）
#[tauri::command]
async fn repair_display(level: Option<u32>) -> Result<String, String> {
    let l = level.unwrap_or(1);
    tauri::async_runtime::spawn_blocking(move || platform::repair_display(l))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

// ---------- 方案B：ADB 联网精细控制（可选增强） ----------

/// ADB 能力探测（是否找到 adb / 已连接设备）
#[tauri::command]
async fn adb_status() -> platform::AdbStatus {
    tauri::async_runtime::spawn_blocking(platform::adb_status)
        .await
        .unwrap_or_else(|_| platform::AdbStatus {
            available: false,
            path: String::new(),
            devices: Vec::new(),
            tip: "后台任务失败，请重试".to_string(),
        })
}

/// 连接显示器的 ADB（addr 形如 192.168.31.216:5555）
#[tauri::command]
async fn adb_connect(addr: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || platform::adb_connect(&addr))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 电源键：待机/唤醒显示器（KEYCODE_POWER）
#[tauri::command]
async fn adb_power(serial: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || platform::adb_power(&serial))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 音量加减
#[tauri::command]
async fn adb_volume(serial: String, up: bool) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || platform::adb_volume(&serial, up))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 通用 adb shell（进阶用法）
#[tauri::command]
async fn adb_shell(serial: String, cmd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || platform::adb_shell(&serial, &cmd))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 一键下载 Android Platform Tools
#[tauri::command]
async fn adb_download() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(platform::adb_download)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
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

/// 菜单栏面板：打开主窗口（窗口操作，留在主线程）
#[tauri::command]
fn open_main(app: tauri::AppHandle) -> Result<(), String> {
    show_main_window(&app);
    Ok(())
}

/// 菜单栏面板：退出（窗口操作，留在主线程）
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

// ================= 显示器健康：自动修复 + 右下角气泡提醒 =================
// 设计：后台线程每 45 秒查一次「设备突然离线」（事件 ID 1010）日志；
// 发现新的显示器掉线 → 弹右下角小窗提醒；若开了自动修复且屏幕没自己回来，
// 就强制重新协商一次显示配置。两个开关都存在 %APPDATA%\Screenguard\health.json。

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct HealthConfig {
    #[serde(default)]
    pub auto_repair: bool,
    #[serde(default = "hc_true")]
    pub notify: bool,
}
fn hc_true() -> bool {
    true
}

fn health_cfg_path() -> std::path::PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    std::path::Path::new(&base)
        .join("Screenguard")
        .join("health.json")
}

pub(crate) fn health_cfg() -> HealthConfig {
    std::fs::read_to_string(health_cfg_path())
        .ok()
        .and_then(|s| serde_json::from_str::<HealthConfig>(&s).ok())
        .unwrap_or(HealthConfig { auto_repair: false, notify: true })
}

fn save_health_cfg(c: &HealthConfig) {
    let p = health_cfg_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string_pretty(c) {
        let _ = std::fs::write(p, s);
    }
}

#[tauri::command]
fn health_get_config() -> HealthConfig {
    health_cfg()
}

#[tauri::command]
fn health_set_config(auto_repair: bool, notify: bool) -> Result<(), String> {
    save_health_cfg(&HealthConfig { auto_repair, notify });
    Ok(())
}

// ---- 气泡内容（notify 窗口创建后自己来取，或由 emit 推送）----
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct NotifyPayload {
    pub icon: String,
    pub title: String,
    pub body: String,
}

fn notify_payload() -> &'static std::sync::Mutex<NotifyPayload> {
    static P: std::sync::OnceLock<std::sync::Mutex<NotifyPayload>> = std::sync::OnceLock::new();
    P.get_or_init(|| std::sync::Mutex::new(NotifyPayload::default()))
}

#[tauri::command]
fn get_notify_payload() -> NotifyPayload {
    notify_payload().lock().map(|g| g.clone()).unwrap_or_default()
}

#[tauri::command]
fn close_notify(app: tauri::AppHandle) {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("notify") {
        let _ = w.hide();
    }
}

/// 自检用：手动弹一条提醒（验证气泡通知是否正常）
#[tauri::command]
fn notify_test(app: tauri::AppHandle) {
    show_notify(
        &app,
        "💡",
        "提醒功能正常",
        "这是测试气泡：以后显示器链路掉线时，这里会第一时间告诉你。",
    );
}

/// 弹一条右下角气泡（不抢焦点，12 秒后自动消失）
fn show_notify(app: &tauri::AppHandle, icon: &str, title: &str, body: &str) {
    use tauri::{Emitter, Manager};
    let p = NotifyPayload {
        icon: icon.to_string(),
        title: title.to_string(),
        body: body.to_string(),
    };
    if let Ok(mut g) = notify_payload().lock() {
        *g = p.clone();
    }
    if let Some(w) = app.get_webview_window("notify") {
        // 定位到主屏右下角（避开任务栏）
        if let Ok(Some(mon)) = w.primary_monitor() {
            let scale = mon.scale_factor();
            let sw = mon.size().width as f64;
            let sh = mon.size().height as f64;
            let ww = 360.0 * scale;
            let wh = 104.0 * scale;
            let x = (sw - ww - 18.0 * scale) as i32;
            let y = (sh - wh - 60.0 * scale) as i32;
            let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
        }
        let _ = w.emit("notify", p.clone());
        let _ = w.show();
        // 窗口首次创建时页面还没加载完（收不到上面那次 emit），
        // 稍后再补发一次，保证气泡里一定有内容
        std::thread::sleep(std::time::Duration::from_millis(260));
        let _ = w.emit("notify", p);
    }
}

/// 后台监控：链路掉线 → 提醒（+ 可选自动修复）
fn spawn_health_watch(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        use std::collections::HashSet;
        use std::time::{Duration, Instant};

        let count_displays = || -> usize {
            platform::mode_detail()
                .lines()
                .filter(|l| l.starts_with("M|"))
                .count()
        };

        // 基线：当前几台屏在用（用于判断掉线后有没有自己回来）
        let mut baseline = count_displays();
        if baseline == 0 {
            std::thread::sleep(Duration::from_secs(20));
            baseline = count_displays();
        }

        let mut seen: HashSet<String> = HashSet::new();
        let mut first = true; // 首次只登记已有事件，避免开机就报历史
        let mut last_fix = Instant::now() - Duration::from_secs(600);

        loop {
            std::thread::sleep(Duration::from_secs(45));
            let cfg = health_cfg();
            let raw = platform::health_events(6); // 近 6 分钟
            let mut fresh: Vec<String> = Vec::new();
            for line in raw.lines() {
                let p: Vec<&str> = line.split('|').collect();
                if p.len() < 6 || p[0] != "E" || p[4] != "1010" {
                    continue; // 只看「设备突然离线」
                }
                let key = format!("{}|{}", p[1], p[5]);
                if seen.insert(key) && !first {
                    fresh.push(p[5].to_string());
                }
            }
            if first {
                first = false;
                continue;
            }
            if fresh.is_empty() {
                continue;
            }
            let disp = fresh.iter().filter(|m| m.contains("DISPLAY")).count();
            if disp == 0 {
                continue; // 非显示设备的事件（USB 存储等）不打扰
            }

            if cfg.notify {
                show_notify(
                    &app,
                    "⚠️",
                    "显示器链路掉线",
                    &format!(
                        "{} 台显示器从系统里消失（同批共 {} 个设备事件）。这类抖动多由 KVM／线材／供电引起。",
                        disp,
                        fresh.len()
                    ),
                );
            }

            if cfg.auto_repair && last_fix.elapsed() > Duration::from_secs(150) {
                std::thread::sleep(Duration::from_secs(6)); // 给系统 6 秒自己恢复的机会
                let now = count_displays();
                if now < baseline {
                    let ok = platform::repair_display(1).is_ok();
                    last_fix = Instant::now();
                    if cfg.notify {
                        show_notify(
                            &app,
                            if ok { "🩺" } else { "❗" },
                            if ok { "已自动修复显示配置" } else { "自动修复未成功" },
                            if ok {
                                "检测到显示器没有自行恢复，已强制重新协商显示配置。若画面仍异常，可在面板点「深度修复」。"
                            } else {
                                "自动重新协商失败，建议在面板手动点「深度修复」，或检查 KVM／线材连接。"
                            },
                        );
                    }
                }
            }
        }
    });
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

            // 显示器健康后台监控（掉线提醒 + 可选自动修复）
            spawn_health_watch(app.handle().clone());
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
            get_batteries,
            apply_color_space,
            match_mac,
            match_ppi,
            rotate_secondary,
            restore_secondary,
            span_video,
            restore_video,
            screen_off,
            screen_wake,
            set_display_power,
            get_display_power,
            display_snapshot,
            health_events,
            mode_detail,
            repair_display,
            health_get_config,
            health_set_config,
            get_notify_payload,
            close_notify,
            notify_test,
            adb_status,
            adb_connect,
            adb_power,
            adb_volume,
            adb_shell,
            adb_download,
            open_main,
            quit_app,
            hide_panel
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
