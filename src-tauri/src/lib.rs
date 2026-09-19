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
    tauri::async_runtime::spawn_blocking(move || platform::set_system_volume_sel(value))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 系统静音开关
#[tauri::command]
async fn set_system_mute(on: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || platform::set_system_mute_sel(on))
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

// ---------- 软件层总亮度 / HDR 同步 / 窗口铺满 / 全局快捷键 ----------

/// 各屏当前总亮度（gamma 反解）：行 G|dev|pct
#[tauri::command]
async fn gamma_get() -> String {
    tauri::async_runtime::spawn_blocking(platform::gamma_get)
        .await
        .unwrap_or_default()
}

/// 设置所有屏的总亮度（40~160，100 = 原始；三通道同曲线，不改色相）
#[tauri::command]
async fn gamma_set(pct: u32) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || platform::gamma_set(pct))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 各屏 HDR 状态：行 H|dev|支持|已启用
#[tauri::command]
async fn hdr_states() -> String {
    tauri::async_runtime::spawn_blocking(platform::hdr_states)
        .await
        .unwrap_or_default()
}

/// 把所有支持 HDR 的屏统一开/关
#[tauri::command]
async fn hdr_set(on: bool) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || platform::hdr_set(on))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 双屏铺满：把当前前台窗口铺满所有屏（不限播放器）
#[tauri::command]
async fn span_foreground() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(platform::span_foreground)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 还原上一次被铺满的窗口
#[tauri::command]
async fn restore_foreground() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(platform::restore_foreground)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 注册全局快捷键（Ctrl+Alt+L：全部屏幕待机/唤醒 切换）
#[tauri::command]
fn hotkey_start() -> Result<String, String> {
    platform::hotkey_start()
}

/// 当前生效的全局快捷键（空 = 未注册）
#[tauri::command]
fn hotkey_label() -> String {
    platform::hotkey_label()
}

/// 跨屏缩放对齐：真正写入逐屏缩放档（先备份）
#[tauri::command]
async fn match_dpi_apply() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(platform::match_dpi_apply)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

#[tauri::command]
async fn match_dpi_restore() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(platform::match_dpi_restore)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

// ---------- v0.3.3：窗口选择 / 音频端点选择 / 一键恢复默认 ----------

/// 可以被铺满的窗口列表："hwnd|宽x高|标题"（不含本程序自己的窗口）
#[tauri::command]
async fn list_windows() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(platform::list_windows)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 铺满指定窗口（按 hwnd，用户从列表里挑）
#[tauri::command]
async fn span_window(hwnd: i64) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || platform::span_window(hwnd))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 所有活动的音频输出端点（含是否可调音量），供「系统控制」里选设备
#[tauri::command]
async fn audio_endpoints() -> Result<Vec<platform::AudioEndpoint>, String> {
    tauri::async_runtime::spawn_blocking(platform::audio_endpoints)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 选定要控制的输出端点（空串 = 跟随系统默认）
#[tauri::command]
async fn set_audio_device(id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || platform::set_audio_device(&id))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 一键恢复默认：把所有「软件层面」的改动还原成基线快照
#[tauri::command]
async fn restore_defaults() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(platform::restore_defaults)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 应用状态（是否退出还原 / 选定的音频端点 / 基线摘要）
#[tauri::command]
fn app_state_json() -> String {
    platform::app_state_json()
}

/// 设置「退出软件时自动还原」
#[tauri::command]
fn set_revert_on_exit(on: bool) -> Result<(), String> {
    platform::set_revert_on_exit(on)
}

/// 小米显示器（REDMI G Pro 27U）当前音量：走它自己的 MiTV 接口（DDC 不通）
#[tauri::command]
async fn mitv_volume_get() -> Result<u32, String> {
    tauri::async_runtime::spawn_blocking(platform::mitv_volume_get)
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 把小米显示器音量调到指定值（按键步进逼近，返回实际读回值）
#[tauri::command]
async fn mitv_volume_set(target: u32) -> Result<u32, String> {
    tauri::async_runtime::spawn_blocking(move || platform::mitv_volume_set(target))
        .await
        .map_err(|e| format!("后台任务失败：{}", e))?
}

/// 小米显示器音量接口是否可用
#[tauri::command]
async fn mitv_available() -> bool {
    tauri::async_runtime::spawn_blocking(platform::mitv_available)
        .await
        .unwrap_or(false)
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
    revert_before_exit();
    app.exit(0);
}

/// 退出前把「软件层面」的改动还原成基线（用户要求：退出软件后自动恢复原样）。
/// 可在状态文件里用 revert_on_exit=false 关掉。
fn revert_before_exit() {
    let js: serde_json::Value =
        serde_json::from_str(&platform::app_state_json()).unwrap_or(serde_json::Value::Null);
    let on = js
        .get("revert_on_exit")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if on {
        let _ = platform::restore_defaults();
    }
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
        // 紧贴主屏工作区右下角。
        // 关键：必须带上显示器自己的**原点**（position）—— 多屏布局里主屏原点常不是 (0,0)，
        // 旧代码只拿 size() 硬算「屏宽-窗宽」，结果算到别的屏/别处去（实测气泡跑到左上角）。
        // 用 Tauri 的显示器信息而不是 PowerShell 的 SPI_GETWORKAREA：
        // PowerShell 进程不是 DPI 感知的，取回的坐标会被虚拟化，反而对不上。
        let scale = w.scale_factor().unwrap_or(1.0);
        let place = |w: &tauri::WebviewWindow| {
            if let Ok(Some(mon)) = w.primary_monitor() {
                let pos = mon.position();
                let size = mon.size();
                let ww = (360.0 * scale) as i32;
                let wh = (104.0 * scale) as i32;
                let pad = (10.0 * scale) as i32;
                let taskbar = (48.0 * scale) as i32; // 主屏任务栏
                let x = pos.x + size.width as i32 - ww - pad;
                let y = pos.y + size.height as i32 - wh - pad - taskbar;
                let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
            } else if let Ok((l, t, r, b)) = platform::primary_work_area() {
                let ww = (360.0 * scale) as i32;
                let wh = (104.0 * scale) as i32;
                let pad = (10.0 * scale) as i32;
                let _ = w.set_position(tauri::PhysicalPosition::new(
                    (r - ww - pad).max(l),
                    (b - wh - pad).max(t),
                ));
            }
        };
        place(&w);
        let _ = w.emit("notify", p.clone());
        let _ = w.show();
        // 窗口首次创建时页面还没加载完（收不到上面那次 emit），
        // 稍后再补发一次，保证气泡里一定有内容；位置也再确认一次。
        std::thread::sleep(std::time::Duration::from_millis(260));
        place(&w);
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
            // 「不依赖界面」的还原入口：铺满之后窗口可能盖住界面上的按钮（实测踩过）
            let unspan = MenuItem::with_id(app, "unspan", "还原铺满的窗口", true, None::<&str>)?;
            let defaults = MenuItem::with_id(app, "defaults", "恢复默认状态", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出（自动还原）", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_main, &unspan, &defaults, &quit])?;

            // 显示/隐藏面板浮窗（右上角、菜单栏下方）
            let toggle_panel = |app_handle: &tauri::AppHandle| {
                if let Some(pw) = app_handle.get_webview_window("panel") {
                    if pw.is_visible().unwrap_or(false) {
                        let _ = pw.hide();
                    } else {
                        if let Ok(Some(mon)) = pw.primary_monitor() {
                            let scale = mon.scale_factor();
                            let x = (mon.size().width as f64 - 372.0 * scale) as i32;
                            // 贴屏幕最顶端（此前留了 28px 偏移，视觉上没靠顶）
                            let y = 0i32;
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
                    "unspan" => {
                        let _ = platform::restore_foreground();
                    }
                    "defaults" => {
                        let _ = platform::restore_defaults();
                    }
                    "quit" => {
                        // 退出前把软件层面的改动还原（用户要求：退出后恢复原样）
                        revert_before_exit();
                        app_handle.exit(0)
                    }
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

            // v0.3.3：启动后采集一次「基线快照」（供「一键恢复默认」和「退出自动还原」）
            // 延后几秒，等显示/音频栈就绪再读，避免采到空值
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(7));
                let _ = platform::capture_baseline();
            });

            // 全局快捷键：Ctrl+Alt+L 切换「全部屏幕待机 / 唤醒」（失败只记日志，不影响启动）
            if let Err(e) = platform::hotkey_start() {
                eprintln!("[screenguard] 全局快捷键未注册：{}", e);
            }
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
            gamma_get,
            gamma_set,
            hdr_states,
            hdr_set,
            span_foreground,
            restore_foreground,
            hotkey_start,
            hotkey_label,
            match_dpi_apply,
            match_dpi_restore,
            list_windows,
            span_window,
            audio_endpoints,
            set_audio_device,
            restore_defaults,
            app_state_json,
            set_revert_on_exit,
            mitv_volume_get,
            mitv_volume_set,
            mitv_available,
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
