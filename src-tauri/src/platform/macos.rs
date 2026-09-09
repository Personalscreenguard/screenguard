use super::{run_cmd, which, DisplayInfo};

const DISPLAYPLACER: &str = "/opt/homebrew/bin/displayplacer";
const BETTERDISPLAY: &str = "/Applications/BetterDisplay.app/Contents/MacOS/BetterDisplay";

pub fn platform_name() -> &'static str {
    "macos"
}

// ===== 开机自启（LaunchAgent）=====
const LA_LABEL: &str = "com.nanyu.screenguard";
const LA_PLIST: &str = "com.nanyu.screenguard.plist";
const LA_BIN: &str = "/Applications/Screenguard.app/Contents/MacOS/screenguard";

fn launch_agents_plist() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(home).join("Library/LaunchAgents").join(LA_PLIST))
}

fn current_uid() -> String {
    std::env::var("UID")
        .unwrap_or_else(|_| run_cmd("id", &["-u"]).unwrap_or_default().trim().to_string())
}

/// LaunchAgent 当前是否已加载到 launchd（在跑或曾跑过）
fn la_loaded(label: &str) -> bool {
    let uid = current_uid();
    run_cmd("launchctl", &["print", &format!("gui/{}/{}", uid, label)]).is_ok()
}

/// 是否已有另一个本程序实例在运行（防 bootstrap 双开）
pub fn another_instance_running() -> bool {
    run_cmd("pgrep", &["-x", "screenguard"])
        .map(|o| o.lines().count() > 1)
        .unwrap_or(false)
}

fn la_plist_xml() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>
"#,
        LA_LABEL, LA_BIN
    )
}

/// 开机自启是否已启用（LaunchAgent plist 存在 = 下次登录会自启）
pub fn autostart_enabled() -> bool {
    launch_agents_plist()
        .map(|p| p.exists())
        .unwrap_or(false)
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let plist = launch_agents_plist().ok_or("无法定位 ~/Library/LaunchAgents")?;
    let uid = current_uid();
    let domain = format!("gui/{}", uid);
    if enabled {
        // 写入 plist（覆盖旧内容，保证指向当前安装路径）
        std::fs::create_dir_all(plist.parent().ok_or("无效路径")?)
            .map_err(|e| format!("创建 LaunchAgents 目录失败：{}", e))?;
        std::fs::write(&plist, la_plist_xml())
            .map_err(|e| format!("写入自启配置失败：{}", e))?;
        // 仅当 agent 尚未加载时才 bootstrap（已加载时重复加载会报错；
        // 新拉起的实例会因单实例自检自动退出，不会双开）
        if !la_loaded(LA_LABEL) {
            run_cmd("launchctl", &["bootstrap", &domain, &plist.to_string_lossy()])
                .map(|_| ())
                .map_err(|e| format!("注册自启失败：{}", e.trim()))?;
        }
        Ok(())
    } else {
        // 只删 plist（下次登录不再自启），不 bootout——
        // bootout 会终止当前由 launchd 拉起的自身实例，等于点"关"把自己杀掉
        match std::fs::remove_file(&plist) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("移除自启配置失败：{}", e)),
        }
    }
}

/// BetterDisplay 必须已安装且正在运行，CLI 才可用
fn betterdisplay_available() -> bool {
    if !std::path::Path::new(BETTERDISPLAY).exists() {
        return false;
    }
    run_cmd("pgrep", &["-x", "BetterDisplay"])
        .map(|o| !o.trim().is_empty())
        .unwrap_or(false)
}

fn displayplacer_available() -> bool {
    std::path::Path::new(DISPLAYPLACER).exists()
}

/// displayplacer 解析出的单屏结构（内部用）
#[derive(Clone, Debug)]
struct Screen {
    id: String,
    /// 原始 Type 描述（如 "27 inch external screen"），仅用于展示兜底
    type_raw: String,
    w: u32,
    h: u32,
    hz: Option<u32>,
    ox: i32,
    oy: i32,
    main: bool,
    degree: u32,
    /// Contextual screen id（可当 CGDirectDisplayID 用）
    ctx: Option<u32>,
}

impl Screen {
    fn res_str(&self) -> String {
        format!("{}x{}", self.w, self.h)
    }
}

/// 解析 displayplacer list 输出（动态，不依赖任何硬编码 id / 分辨率）
fn parse_screens() -> Vec<Screen> {
    let raw = run_cmd(DISPLAYPLACER, &["list"]).unwrap_or_default();
    let mut result = Vec::new();
    let mut cur: Option<Screen> = None;

    for line in raw.lines() {
        let t = line.trim();
        if let Some(v) = t.strip_prefix("Persistent screen id:") {
            if let Some(s) = cur.take() {
                result.push(s);
            }
            cur = Some(Screen {
                id: v.trim().to_string(),
                type_raw: String::new(),
                w: 0,
                h: 0,
                hz: None,
                ox: 0,
                oy: 0,
                main: false,
                degree: 0,
                ctx: None,
            });
        } else if let Some(s) = cur.as_mut() {
            if let Some(v) = t.strip_prefix("Contextual screen id:") {
                s.ctx = v.trim().parse::<u32>().ok();
            } else if let Some(v) = t.strip_prefix("Type:") {
                s.type_raw = v.trim().to_string();
            } else if let Some(v) = t.strip_prefix("Resolution:") {
                let v = v.trim();
                if let Some((a, b)) = v.split_once('x') {
                    s.w = a.trim().parse().unwrap_or(0);
                    s.h = b.trim().parse().unwrap_or(0);
                }
            } else if let Some(v) = t.strip_prefix("Hertz:") {
                s.hz = v.trim().parse::<u32>().ok();
            } else if let Some(v) = t.strip_prefix("Origin:") {
                // " (0,0)" 或 " (1920,0) - main display"
                s.main = v.contains("main display");
                if let Some(open) = v.find('(') {
                    if let Some(close) = v.find(')') {
                        let inner = &v[open + 1..close];
                        if let Some((x, y)) = inner.split_once(',') {
                            s.ox = x.trim().parse().unwrap_or(0);
                            s.oy = y.trim().parse().unwrap_or(0);
                        }
                    }
                }
            } else if let Some(v) = t.strip_prefix("Rotation:") {
                s.degree = v.trim().parse().unwrap_or(0);
            }
        }
    }
    if let Some(s) = cur.take() {
        result.push(s);
    }
    result
}

/// 主屏 = 带 main display 标记的；副屏 = 其余第一块（本项目为双屏工具）
fn main_secondary(screens: &[Screen]) -> Result<(&Screen, &Screen), String> {
    let main = screens
        .iter()
        .find(|s| s.main)
        .ok_or_else(|| "未找到主屏（请检查 displayplacer 输出）".to_string())?;
    let sec = screens
        .iter()
        .find(|s| !s.main)
        .ok_or_else(|| "需要主屏 + 副屏各一块才能使用此功能".to_string())?;
    Ok((main, sec))
}

/// 界面名字：只按角色命名，不猜品牌/尺寸（EDID 上报尺寸可能失真）
fn role_name(is_main: bool) -> String {
    if is_main {
        "主屏".to_string()
    } else {
        "副屏".to_string()
    }
}

pub fn get_displays() -> Vec<DisplayInfo> {
    if !displayplacer_available() {
        return Vec::new();
    }
    let screens = parse_screens();
    let bd = betterdisplay_available();
    screens
        .into_iter()
        .map(|s| {
            let (b, vol) = match (s.ctx, bd) {
                (Some(c), true) => (read_ddc_brightness(c), read_ddc_volume(c)),
                _ => (None, None),
            };
            let prof = if bd { s.ctx.and_then(read_color_profile) } else { None };
            DisplayInfo {
                id: s.id.clone(),
                name: role_name(s.main),
                resolution: s.res_str(),
                hz: s.hz,
                main: s.main,
                connected: true,
                ddc: b.is_some(),
                brightness: b,
                volume: vol,
                color_profile: prof,
                ddc_id: s.ctx,
            }
        })
        .collect()
}

// BetterDisplay 的 get 返回小数亮度（0-1），换算成 0-100
fn read_ddc_brightness(cgid: u32) -> Option<u32> {
    let out = run_cmd(BETTERDISPLAY, &["get", &ddctl(cgid), "-hardwareBrightness"]).ok()?;
    out.trim()
        .parse::<f64>()
        .ok()
        .map(|f| (f * 100.0).round() as u32)
}
fn read_ddc_volume(cgid: u32) -> Option<u32> {
    let out = run_cmd(BETTERDISPLAY, &["get", &ddctl(cgid), "-volume"]).ok()?;
    out.trim()
        .parse::<f64>()
        .ok()
        .map(|f| (f * 100.0).round() as u32)
}
fn ddctl(cgid: u32) -> String {
    format!("-displayID={}", cgid)
}

/// 按 displayplacer 持久 id 找到对应显示器的 DDC 句柄
fn find_ddc_id(display_id: &str) -> Result<u32, String> {
    let disp = get_displays()
        .into_iter()
        .find(|d| d.id == display_id)
        .ok_or("未找到显示器")?;
    disp.ddc_id
        .ok_or("该显示器不支持 DDC/CI（或 BetterDisplay 未运行）".into())
}

fn ensure_betterdisplay() -> Result<(), String> {
    if betterdisplay_available() {
        Ok(())
    } else if std::path::Path::new(BETTERDISPLAY).exists() {
        Err("BetterDisplay 未在运行，请先启动它（菜单栏/LaunchAgent）".into())
    } else {
        Err("未安装 BetterDisplay（DDC/CI 控制依赖它）".into())
    }
}

/// 设置亮度 0-100
pub fn set_brightness(display_id: &str, value: u32) -> Result<(), String> {
    ensure_betterdisplay()?;
    let cgid = find_ddc_id(display_id)?;
    let frac = format!("{:.4}", (value.clamp(0, 100) as f64) / 100.0);
    run_cmd(
        BETTERDISPLAY,
        &[
            "set",
            &ddctl(cgid),
            &format!("-hardwareBrightness={}", frac),
        ],
    )
    .map(|_| ())
}

/// 设置音量 0-100
pub fn set_volume(display_id: &str, value: u32) -> Result<(), String> {
    ensure_betterdisplay()?;
    let cgid = find_ddc_id(display_id)?;
    let frac = format!("{:.4}", (value.clamp(0, 100) as f64) / 100.0);
    run_cmd(
        BETTERDISPLAY,
        &["set", &ddctl(cgid), &format!("-volume={}", frac)],
    )
    .map(|_| ())
}

// ===== 色彩 / 布局 =====
/// 读某显示器当前 ColorSync profile 文件名
fn read_color_profile(cgid: u32) -> Option<String> {
    let out = run_cmd(BETTERDISPLAY, &["get", &ddctl(cgid), "-colorProfileURL"]).ok()?;
    out.trim()
        .rsplit('/')
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn space_profile_url(space: &str) -> &'static str {
    match space {
        "p3" => "file:///System/Library/ColorSync/Profiles/Display%20P3.icc",
        "adobeRGB" => "file:///System/Library/ColorSync/Profiles/AdobeRGB1998.icc",
        _ => "file:///System/Library/ColorSync/Profiles/sRGB%20Profile.icc",
    }
}

/// 把目标色彩配置应用到所有显示器（软件侧可落地的"色彩同步"）
pub fn apply_color_space(space: &str) -> Result<(), String> {
    ensure_betterdisplay()?;
    let profile = space_profile_url(space);
    let displays = get_displays();
    let mut applied = 0;
    let mut last_err = String::new();
    for d in &displays {
        if let Some(cgid) = d.ddc_id {
            match run_cmd(
                BETTERDISPLAY,
                &[
                    "set",
                    &ddctl(cgid),
                    &format!("-colorProfileURL={}", profile),
                ],
            ) {
                Ok(_) => applied += 1,
                Err(e) => last_err = e,
            }
        }
    }
    if applied == 0 {
        Err(if last_err.is_empty() {
            "未能应用色彩配置（未找到可控制的显示器）".into()
        } else {
            last_err
        })
    } else {
        Ok(())
    }
}

pub fn match_mac() -> Result<(), String> {
    // Apple 内建屏为 Display P3，将外接屏全部对齐到 Display P3（软件可落地的"对齐 Mac"）
    apply_color_space("p3")
}

/// 用 displayplacer 完整设置两块屏的布局（主屏保持现状，只动副屏的 res/方向）
fn apply_sec_layout(main: &Screen, sec: &Screen, sec_res: &str, sec_deg: u32) -> Result<(), String> {
    let main_spec = format!(
        "id:{} res:{} hz:{} color_depth:8 enabled:true scaling:on origin:({},{}) degree:0",
        main.id,
        main.res_str(),
        main.hz.unwrap_or(60),
        main.ox,
        main.oy
    );
    let sec_spec = format!(
        "id:{} res:{} hz:{} color_depth:8 enabled:true scaling:on origin:({},{}) degree:{}",
        sec.id, sec_res, sec.hz.unwrap_or(60), sec.ox, sec.oy, sec_deg
    );
    run_cmd(DISPLAYPLACER, &[&main_spec, &sec_spec]).map(|_| ())
}

fn is_standard_hidpi(sec: &Screen) -> bool {
    // 副屏标准 2x 档：横屏 1920x1080 / 竖屏 1080x1920（4K 面板 @2x）
    if sec.degree != 0 {
        sec.w == 1080 && sec.h == 1920
    } else {
        sec.w == 1920 && sec.h == 1080
    }
}

/// "窗口拖到另一屏不变小"：让副屏处于与主屏一致的 2x 逻辑档位。
/// 只按副屏当前方向恢复标准 2x 分辨率，绝不动主屏、绝不把 4K 面板降到 1080p 物理档。
pub fn match_ppi() -> Result<(), String> {
    let screens = parse_screens();
    if screens.is_empty() {
        return Err("displayplacer 不可用或没有显示器".into());
    }
    let (main, sec) = main_secondary(&screens)?;
    // 主屏必须已是 1080p 逻辑档（27" 4K @2x），否则无法保证跨屏一致
    if main.w != 1920 || main.h != 1080 {
        return Err(format!(
            "主屏当前逻辑分辨率为 {}，请先将其设为 1920x1080（2x 档）再匹配",
            main.res_str()
        ));
    }
    if is_standard_hidpi(sec) {
        return Ok(()); // 已就位，不动
    }
    let (res, deg) = if sec.degree != 0 {
        ("1080x1920".to_string(), 270u32)
    } else {
        ("1920x1080".to_string(), 0u32)
    };
    match apply_sec_layout(main, sec, &res, deg) {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("匹配失败：{}", e.trim())),
    }
}

/// 副屏横/竖屏切换（再点一次转回，可恢复）
pub fn rotate_secondary() -> Result<(), String> {
    let screens = parse_screens();
    let (main, sec) = main_secondary(&screens)?;
    if sec.degree == 0 {
        apply_sec_layout(main, sec, "1080x1920", 270) // 横屏 -> 竖屏
    } else {
        apply_sec_layout(main, sec, "1920x1080", 0) // 竖屏 -> 横屏
    }
}

/// 恢复副屏为竖屏（安全复位）
pub fn restore_secondary() -> Result<(), String> {
    let screens = parse_screens();
    let (main, sec) = main_secondary(&screens)?;
    apply_sec_layout(main, sec, "1080x1920", 270)
}

/// 所有已连接屏幕的包围盒（逻辑坐标），用于跨屏铺满
fn union_bounds(screens: &[Screen]) -> Option<(i32, i32, i32, i32)> {
    if screens.is_empty() {
        return None;
    }
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for s in screens {
        min_x = min_x.min(s.ox);
        min_y = min_y.min(s.oy);
        max_x = max_x.max(s.ox + s.w as i32);
        max_y = max_y.max(s.oy + s.h as i32);
    }
    Some((min_x, min_y, max_x, max_y))
}

// ===== 视频跨屏（VLC）= 按当前真实布局铺满所有屏的并集 =====
pub fn span_video() -> Result<(), String> {
    let installed =
        which("vlc") || std::path::Path::new("/Applications/VLC.app/Contents/MacOS/VLC").exists();
    if !installed {
        return Err("未安装 VLC".into());
    }
    let screens = parse_screens();
    if screens.len() < 2 {
        return Err("需要至少两块屏幕才能双屏铺满".into());
    }
    let (x0, y0, x1, y1) = union_bounds(&screens).ok_or("无法计算屏幕布局")?;

    // 用 osascript 统一处理：activate 启动 VLC → 等主窗口出现 → 铺到包围盒
    let script = format!(
        r#"
on run
  tell application "VLC" to activate
  delay 2.0
  set bx1 to {}
  set by1 to {}
  set bx2 to {}
  set by2 to {}
  tell application "VLC"
    set targetWindow to missing value
    set maxArea to 0
    repeat 30 times
      try
        set targetWindow to missing value
        set maxArea to 0
        repeat with wnd in windows
          try
            set {{x1, y1, x2, y2}} to bounds of wnd
            set area to (x2 - x1) * (y2 - y1)
            if area > maxArea then
              set maxArea to area
              set targetWindow to wnd
            end if
          end try
        end repeat
        if targetWindow is not missing value then
          set bounds of targetWindow to {{bx1, by1, bx2, by2}}
          delay 0.4
        end if
      end try
    end repeat
  end tell
end run
"#,
        x0, y0, x1, y1
    );
    run_cmd("osascript", &["-e", &script])?;
    Ok(())
}

pub fn restore_video() -> Result<(), String> {
    // 把 VLC 主窗口恢复为铺满主屏（退出跨屏状态）
    let screens = parse_screens();
    let main = screens
        .iter()
        .find(|s| s.main)
        .ok_or("未找到主屏")?;
    let x0 = main.ox;
    let y0 = main.oy;
    let x1 = x0 + main.w as i32;
    let y1 = y0 + main.h as i32;
    let script = format!(
        r#"
on run
  tell application "VLC" to activate
  delay 1.0
  set bx1 to {}
  set by1 to {}
  set bx2 to {}
  set by2 to {}
  tell application "VLC"
    set targetWindow to missing value
    set maxArea to 0
    repeat 10 times
      try
        set targetWindow to missing value
        set maxArea to 0
        repeat with wnd in windows
          try
            set {{x1, y1, x2, y2}} to bounds of wnd
            set area to (x2 - x1) * (y2 - y1)
            if area > maxArea then
              set maxArea to area
              set targetWindow to wnd
            end if
          end try
        end repeat
        if targetWindow is not missing value then
          set bounds of targetWindow to {{bx1, by1, bx2, by2}}
          delay 0.4
        end if
      end try
    end repeat
  end tell
end run
"#,
        x0, y0, x1, y1
    );
    run_cmd("osascript", &["-e", &script])?;
    Ok(())
}
