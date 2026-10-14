use super::{run_cmd, which, DisplayInfo};
use serde::{Deserialize, Serialize};

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

fn displayplacer_path() -> Option<&'static str> {
    [
        "/opt/homebrew/bin/displayplacer",
        "/usr/local/bin/displayplacer",
        "/opt/local/bin/displayplacer",
    ]
    .into_iter()
    .find(|p| std::path::Path::new(p).exists())
    .or_else(|| which("displayplacer").then_some("displayplacer"))
}

fn run_displayplacer(args: &[&str]) -> Result<String, String> {
    let path = displayplacer_path().ok_or_else(|| {
        "找不到 displayplacer。请安装：brew install displayplacer".to_string()
    })?;
    run_cmd(path, args).map_err(|e| format!("displayplacer 执行失败：{}", e.trim()))
}

fn displayplacer_available() -> bool {
    displayplacer_path().is_some()
}

fn mode_property<'a>(prop: &'a str, prefix: &str, props: &'a [&'a str], index: usize) -> Option<&'a str> {
    prop.strip_prefix(prefix)
        .filter(|v| !v.is_empty())
        .or_else(|| if prop == prefix { props.get(index + 1).copied() } else { None })
}

#[derive(Clone, Debug)]
struct ScreenMode {
    index: u32,
    w: u32,
    h: u32,
    hz: Option<u32>,
    depth: u32,
    scaling: bool,
    current: bool,
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
    enabled: bool,
    main: bool,
    degree: u32,
    scaling: bool,
    color_depth: u32,
    modes: Vec<ScreenMode>,
    /// Mode selected for a pending layout. Parsed screens leave this empty and
    /// use their explicitly marked current mode instead of mixing backing pixels
    /// from `Resolution:` with desktop-coordinate mode dimensions.
    requested_mode: Option<ScreenMode>,
    /// Contextual screen id（可当 CGDirectDisplayID 用）
    ctx: Option<u32>,
}

impl Screen {
    fn res_str(&self) -> String {
        format!("{}x{}", self.w, self.h)
    }

    fn active_mode(&self) -> Option<&ScreenMode> {
        self.modes.iter().find(|m| m.current)
    }

    fn layout_mode(&self) -> Option<&ScreenMode> {
        self.requested_mode.as_ref().or_else(|| self.active_mode())
    }

    /// displayplacer mode rows and origins use desktop logical points, while its
    /// Resolution line is CGDisplayPixelsWide/High (backing pixels).
    fn frame_w(&self) -> u32 {
        self.layout_mode().map(|m| m.w).unwrap_or_else(|| if self.scaling { self.w / 2 } else { self.w })
    }

    fn frame_h(&self) -> u32 {
        self.layout_mode().map(|m| m.h).unwrap_or_else(|| if self.scaling { self.h / 2 } else { self.h })
    }

    fn layout_resolution(&self) -> (u32, u32) {
        self.layout_mode().map(|m| (m.w, m.h)).unwrap_or_else(|| {
            if self.scaling { (self.w / 2, self.h / 2) } else { (self.w, self.h) }
        })
    }
}

/// 解析 displayplacer list 输出（动态，不依赖任何硬编码 id / 分辨率）
fn parse_screens() -> Vec<Screen> {
    let raw = run_displayplacer(&["list"]).unwrap_or_default();
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
                enabled: true,
                main: false,
                degree: 0,
                scaling: false,
                color_depth: 8,
                modes: Vec::new(),
                requested_mode: None,
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
            } else if let Some(v) = t.strip_prefix("Color Depth:") {
                s.color_depth = v.trim().parse::<u32>().unwrap_or(8);
            } else if let Some(v) = t.strip_prefix("Scaling:") {
                s.scaling = v.trim().eq_ignore_ascii_case("on");
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
            } else if let Some(v) = t.strip_prefix("Enabled:") {
                s.enabled = v.trim().eq_ignore_ascii_case("true");
            } else if let Some(v) = t.strip_prefix("Rotation:") {
                s.degree = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = t.strip_prefix("mode ") {
                if let Some((n, props)) = v.split_once(':') {
                    let index = n.trim().parse::<u32>().ok();
                    let mut w = 0;
                    let mut h = 0;
                    let mut hz = None;
                    let mut depth = 8;
                    let mut scaling = false;
                    let current = t.contains("<-- current mode");
                    let props: Vec<&str> = props.split_whitespace().collect();
                    let mut pi = 0;
                    while pi < props.len() {
                        let prop = props[pi];
                        if let Some(res) = mode_property(prop, "res:", &props, pi) {
                            if let Some((a, b)) = res.split_once('x') {
                                w = a.trim().parse().unwrap_or(0);
                                h = b.trim().parse().unwrap_or(0);
                            }
                        } else if let Some(v) = mode_property(prop, "hz:", &props, pi) {
                            hz = v.parse::<f64>().ok().map(|n| n.round() as u32);
                        } else if let Some(v) = mode_property(prop, "color_depth:", &props, pi) {
                            depth = v.parse().unwrap_or(8);
                        } else if let Some(v) = mode_property(prop, "scaling:", &props, pi) {
                            scaling = v.eq_ignore_ascii_case("on");
                        }
                        pi += if prop.ends_with(':') && pi + 1 < props.len() { 2 } else { 1 };
                    }
                    if let Some(index) = index {
                        if w > 0 && h > 0 {
                            s.modes.push(ScreenMode { index, w, h, hz, depth, scaling, current });
                        }
                    }
                }
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
    let active: Vec<&Screen> = screens.iter().filter(|s| s.enabled).collect();
    let main = active
        .iter()
        .copied()
        .find(|s| s.main)
        .ok_or_else(|| "未找到主屏（请检查 displayplacer 输出）".to_string())?;
    let sec = active
        .iter()
        .copied()
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
    let (main_w, main_h) = main.layout_resolution();
    let main_hz = main.hz.map(|v| format!(" hz:{}", v)).unwrap_or_default();
    let sec_hz = sec.hz.map(|v| format!(" hz:{}", v)).unwrap_or_default();
    let main_spec = format!(
        "id:{} res:{}x{}{} color_depth:{} enabled:true scaling:{} origin:({},{}) degree:{}",
        main.id,
        main_w,
        main_h,
        main_hz,
        main.color_depth.max(8),
        if main.scaling { "on" } else { "off" },
        main.ox,
        main.oy,
        main.degree
    );
    let sec_spec = format!(
        "id:{} res:{}{} color_depth:8 enabled:true scaling:on origin:({},{}) degree:{}",
        sec.id, sec_res, sec_hz, sec.ox, sec.oy, sec_deg
    );
    run_displayplacer(&[&main_spec, &sec_spec]).map(|_| ())
}

// ===== 原生分辨率探测：目标档位全部动态推导，不硬编码任何分辨率 =====
/// 从 "3840 x 2160 (4K UHD)" / "1920 x 1080 @ 60.00Hz" 之类的字符串里取出宽高
fn parse_wh(s: &str) -> Option<(u32, u32)> {
    let t: Vec<&str> = s.split_whitespace().collect();
    for w in t.windows(3) {
        if w[1].eq_ignore_ascii_case("x") {
            if let (Ok(a), Ok(b)) = (w[0].parse::<u32>(), w[2].parse::<u32>()) {
                return Some((a, b));
            }
        }
    }
    None
}

/// (原生宽, 原生高, 逻辑宽, 逻辑高)
type SpScreen = (u32, u32, u32, u32);

/// 解析 `system_profiler SPDisplaysDataType`，得到每块屏的原生像素与逻辑像素。
/// 解析失败返回空表 —— 调用方回退到「跟随主屏逻辑分辨率」，功能不会因此不可用。
fn screens_native() -> Vec<SpScreen> {
    let raw = match run_cmd("system_profiler", &["SPDisplaysDataType"]) {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<SpScreen> = Vec::new();
    let mut native: Option<(u32, u32)> = None;
    let mut logical: Option<(u32, u32)> = None;
    for line in raw.lines() {
        let indent = line.len() - line.trim_start().len();
        let t = line.trim();
        // 8 空格缩进且以冒号结尾 = 新的一块显示器，落盘上一块
        if indent == 8 && t.ends_with(':') && !t.is_empty() {
            if let Some(n) = native {
                let l = logical.unwrap_or(n);
                out.push((n.0, n.1, l.0, l.1));
            }
            native = None;
            logical = None;
            continue;
        }
        if let Some(v) = t.strip_prefix("Resolution:") {
            if let Some(wh) = parse_wh(v) {
                // 外接 1x 屏此处就是当前分辨率；内建 Retina 屏是原生像素
                native = Some(wh);
            }
        } else if let Some(v) = t.strip_prefix("UI Looks like:") {
            if let Some(wh) = parse_wh(v) {
                logical = Some(wh);
            }
        }
    }
    if let Some(n) = native {
        let l = logical.unwrap_or(n);
        out.push((n.0, n.1, l.0, l.1));
    }
    out
}

/// 在探测表里按「逻辑分辨率」匹配同一块屏（横竖屏都试）
fn find_native<'a>(sps: &'a [SpScreen], s: &Screen) -> Option<&'a SpScreen> {
    sps.iter()
        .find(|(_, _, lw, lh)| (*lw, *lh) == (s.w, s.h) || (*lh, *lw) == (s.w, s.h))
}

/// 副屏的「2x 逻辑档」目标 = 原生像素 ÷ 2（方向随横竖屏）。
/// 5K 面板得 2560x1440、4K 面板得 1920x1080、2K 面板得 1280x720，
/// 不再硬编码 1920x1080。探不到原生信息时回退为「跟随主屏逻辑分辨率」。
fn hidpi_target(sps: &[SpScreen], main: &Screen, sec: &Screen, portrait: bool) -> (String, u32) {
    let (lw, lh) = match find_native(sps, sec) {
        Some((nw, nh, _, _)) => ((nw / 2).max(1), (nh / 2).max(1)),
        None => (main.w.max(1), main.h.max(1)),
    };
    if portrait {
        (format!("{}x{}", lh, lw), 270)
    } else {
        (format!("{}x{}", lw, lh), 0)
    }
}

/// 主屏是否已处于 2x 档（原生像素 = 逻辑像素 × 2）；探不到时返回 None（不阻塞）
fn main_at_2x(sps: &[SpScreen], main: &Screen) -> Option<bool> {
    find_native(sps, main).map(|(nw, nh, _, _)| (main.w, main.h) == (nw / 2, nh / 2))
}

/// "窗口拖到另一屏不变小"：让副屏处于与主屏一致的 2x 逻辑档位。
/// 只按副屏原生像素推导目标档、只动副屏，绝不动主屏、绝不把高分辨率面板降到低物理档。
pub fn match_ppi() -> Result<(), String> {
    let screens = parse_screens();
    if screens.is_empty() {
        return Err("displayplacer 不可用或没有显示器".into());
    }
    let (main, sec) = main_secondary(&screens)?;
    let sps = screens_native();
    if let Some(false) = main_at_2x(&sps, main) {
        return Err(format!(
            "主屏当前逻辑分辨率为 {}，不是其原生像素的 2x 档；请先在「系统设置 → 显示器」把它设为 2x 档再匹配",
            main.res_str()
        ));
    }
    let portrait = sec.degree != 0;
    let (target, deg) = hidpi_target(&sps, main, sec, portrait);
    if format!("{}x{}", sec.w, sec.h) == target && sec.degree == deg {
        return Ok(()); // 已就位，不动
    }
    match apply_sec_layout(main, sec, &target, deg) {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("匹配失败：{}", e.trim())),
    }
}

/// 副屏横/竖屏切换（再点一次转回，可恢复）
pub fn rotate_secondary() -> Result<(), String> {
    let screens = parse_screens();
    let (main, sec) = main_secondary(&screens)?;
    let sps = screens_native();
    let portrait = sec.degree == 0; // 当前横屏 → 转竖屏
    let (target, deg) = hidpi_target(&sps, main, sec, portrait);
    apply_sec_layout(main, sec, &target, deg)
}

/// 恢复副屏为竖屏（安全复位）
pub fn restore_secondary() -> Result<(), String> {
    let screens = parse_screens();
    let (main, sec) = main_secondary(&screens)?;
    if sec.degree != 0 {
        return Ok(()); // 已是竖屏，不动
    }
    let sps = screens_native();
    let (target, deg) = hidpi_target(&sps, main, sec, true);
    apply_sec_layout(main, sec, &target, deg)
}

/// 所有已连接屏幕的包围盒（逻辑坐标），用于跨屏铺满
fn union_bounds(screens: &[Screen]) -> Option<(i32, i32, i32, i32)> {
    let active: Vec<&Screen> = screens.iter().filter(|s| s.enabled).collect();
    if active.is_empty() || active.iter().any(|s| s.layout_mode().is_none()) {
        return None;
    }
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for s in active {
        min_x = min_x.min(s.ox);
        min_y = min_y.min(s.oy);
        max_x = max_x.max(s.ox + s.frame_w() as i32);
        max_y = max_y.max(s.oy + s.frame_h() as i32);
    }
    Some((min_x, min_y, max_x, max_y))
}

// ===== 双屏逻辑高度对齐：保存完整布局，切换后重新枚举并校验 =====

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SavedScreenLayout {
    id: String,
    #[serde(default = "default_enabled")]
    enabled: bool,
    w: u32,
    h: u32,
    #[serde(default)]
    mode_index: Option<u32>,
    #[serde(default)]
    hz: Option<u32>,
    depth: u32,
    scaling: bool,
    ox: i32,
    oy: i32,
    degree: u32,
    main: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SavedLayout {
    screens: Vec<SavedScreenLayout>,
}

fn default_enabled() -> bool {
    true
}

fn app_support_dir() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "无法定位当前用户主目录".to_string())?;
    let dir = std::path::PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Screenguard");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 ScreenGuard 数据目录失败：{}", e))?;
    Ok(dir)
}

fn layout_backup_path() -> Result<std::path::PathBuf, String> {
    Ok(app_support_dir()?.join("display-layout-backup.json"))
}

fn save_json_atomic(path: &std::path::Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| format!("序列化备份失败：{}", e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("写入备份失败：{}", e))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("提交备份失败：{}", e))
}

fn saved_layout(screens: &[Screen]) -> SavedLayout {
    SavedLayout {
        screens: screens
            .iter()
            .map(|s| SavedScreenLayout {
                id: s.id.clone(),
                enabled: s.enabled,
                w: s.layout_resolution().0,
                h: s.layout_resolution().1,
                mode_index: s.active_mode().map(|m| m.index),
                hz: s.hz,
                depth: s.color_depth.max(8),
                scaling: s.scaling,
                ox: s.ox,
                oy: s.oy,
                degree: s.degree,
                main: s.main,
            })
            .collect(),
    }
}

fn save_original_layout(screens: &[Screen]) -> Result<(), String> {
    let path = layout_backup_path()?;
    // Keep the first pre-change snapshot until the user explicitly restores it.
    if path.exists() {
        return Ok(());
    }
    save_json_atomic(&path, &saved_layout(screens))
}

fn clear_layout_backup() -> Result<(), String> {
    let path = layout_backup_path()?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除对齐失败后的布局备份失败：{}", e)),
    }
}

fn screen_spec(s: &Screen) -> String {
    if !s.enabled {
        return format!("id:{} enabled:false", s.id);
    }
    if let Some(mode) = &s.requested_mode {
        // Address the selected row directly so identical logical dimensions with
        // different Retina scaling profiles cannot resolve to a different mode.
        return format!("id:{} mode:{} enabled:true origin:({},{}) degree:{}", s.id, mode.index, s.ox, s.oy, s.degree);
    }
    let (w, h) = s.layout_resolution();
    let hz = s.hz.map(|v| format!(" hz:{}", v)).unwrap_or_default();
    format!(
        "id:{} res:{}{} color_depth:{} enabled:true scaling:{} origin:({},{}) degree:{}",
        s.id,
        format!("{}x{}", w, h),
        hz,
        s.color_depth.max(8),
        if s.scaling { "on" } else { "off" },
        s.ox,
        s.oy,
        s.degree
    )
}

fn screen_layout_matches(actual: &Screen, wanted: &Screen) -> bool {
    if actual.id != wanted.id || actual.enabled != wanted.enabled {
        return false;
    }
    if !wanted.enabled {
        return true;
    }
    let mode_matches = match &wanted.requested_mode {
        Some(requested) => actual.active_mode().map(|m| m.index) == Some(requested.index),
        None => actual.layout_mode().map(|m| (m.w, m.h, m.hz, m.depth, m.scaling))
            == wanted.layout_mode().map(|m| (m.w, m.h, m.hz, m.depth, m.scaling)),
    };
    mode_matches
        && actual.ox == wanted.ox
        && actual.oy == wanted.oy
        && actual.degree == wanted.degree
        && actual.main == wanted.main
}

fn layouts_match(actual: &[Screen], wanted: &[Screen]) -> bool {
    actual.len() == wanted.len()
        && wanted.iter().all(|w| actual.iter().find(|a| a.id == w.id).is_some_and(|a| screen_layout_matches(a, w)))
}

fn apply_screen_layout(screens: &[Screen]) -> Result<(), String> {
    if screens.is_empty() {
        return Err("没有可应用的显示器布局".into());
    }
    if layouts_match(&parse_screens(), screens) {
        return Ok(());
    }
    let specs: Vec<String> = screens.iter().map(screen_spec).collect();
    let args: Vec<&str> = specs.iter().map(String::as_str).collect();
    run_displayplacer(&args).map(|_| ())
}

fn restore_saved_layout(saved: &SavedLayout) -> Result<(), String> {
    let connected = parse_screens();
    let connected_ids: std::collections::HashSet<&str> =
        connected.iter().map(|s| s.id.as_str()).collect();
    let missing: Vec<&str> = saved
        .screens
        .iter()
        .filter(|s| !connected_ids.contains(s.id.as_str()))
        .map(|s| s.id.as_str())
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "原布局包含当前未连接的显示器（{}），请接回显示器后再还原；备份已保留",
            missing.join("、")
        ));
    }
    let already_restored = saved.screens.iter().all(|wanted| {
        connected.iter().find(|s| s.id == wanted.id).is_some_and(|actual| {
            actual.enabled == wanted.enabled
                && (!wanted.enabled || (actual.layout_mode().is_some()
                    && actual.frame_w() == wanted.w
                    && actual.frame_h() == wanted.h
                    && wanted.mode_index.map_or(true, |index| actual.active_mode().map(|m| m.index) == Some(index))
                    && actual.hz == wanted.hz
                    && actual.color_depth == wanted.depth
                    && actual.ox == wanted.ox
                    && actual.oy == wanted.oy
                    && actual.degree == wanted.degree
                    && actual.scaling == wanted.scaling
                    && actual.main == wanted.main))
        })
    });
    if already_restored {
        return Ok(());
    }
    let specs: Vec<String> = saved
        .screens
        .iter()
        .map(|s| {
            if !s.enabled {
                return format!("id:{} enabled:false", s.id);
            }
            if let Some(mode_index) = s.mode_index {
                return format!(
                    "id:{} mode:{} enabled:true origin:({},{}) degree:{}",
                    s.id, mode_index, s.ox, s.oy, s.degree
                );
            }
            format!(
                "id:{} res:{}x{}{} color_depth:{} enabled:true scaling:{} origin:({},{}) degree:{}",
                s.id,
                s.w,
                s.h,
                s.hz.map(|v| format!(" hz:{}", v)).unwrap_or_default(),
                s.depth,
                if s.scaling { "on" } else { "off" },
                s.ox,
                s.oy,
                s.degree
            )
        })
        .collect();
    let args: Vec<&str> = specs.iter().map(String::as_str).collect();
    run_displayplacer(&args)?;
    std::thread::sleep(std::time::Duration::from_millis(900));
    let after = parse_screens();
    for wanted in &saved.screens {
        let actual = after.iter().find(|s| s.id == wanted.id).ok_or_else(|| {
            format!("还原后找不到显示器 {}；布局备份已保留", wanted.id)
        })?;
        if actual.enabled != wanted.enabled
            || (wanted.enabled && (actual.frame_w() != wanted.w
            || actual.frame_h() != wanted.h
            || wanted.mode_index.is_some_and(|index| actual.active_mode().map(|m| m.index) != Some(index))
            || actual.hz != wanted.hz
            || actual.color_depth != wanted.depth
            || actual.ox != wanted.ox
            || actual.oy != wanted.oy
            || actual.degree != wanted.degree
            || actual.scaling != wanted.scaling
            || actual.main != wanted.main))
        {
            return Err(format!(
                "显示器 {} 未恢复到原布局：目标 {}x{}@{}、位置 ({},{}), 方向 {}°；实际 {}x{}@{}、位置 ({},{}), 方向 {}°。备份已保留",
                wanted.id, wanted.w, wanted.h, hz_label(wanted.hz), wanted.ox, wanted.oy, wanted.degree,
                actual.frame_w(), actual.frame_h(), hz_label(actual.hz), actual.ox, actual.oy, actual.degree
            ));
        }
    }
    Ok(())
}

fn hz_label(hz: Option<u32>) -> String {
    hz.map(|v| v.to_string()).unwrap_or_else(|| "默认".into())
}

fn option_delta(a: Option<u32>, b: Option<u32>) -> f64 {
    match (a, b) {
        (Some(a), Some(b)) => a.abs_diff(b) as f64,
        _ => 0.0,
    }
}

pub fn align_restore_modes() -> Result<String, String> {
    let path = layout_backup_path()?;
    if !path.exists() {
        return Ok("没有记录过原始显示布局，无需还原".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("读取布局备份失败：{}", e))?;
    let saved: SavedLayout = serde_json::from_slice(&bytes)
        .map_err(|e| format!("布局备份损坏，未改动当前显示：{}", e))?;
    restore_saved_layout(&saved)?;
    std::fs::remove_file(&path).map_err(|e| format!("布局已还原，但删除备份失败：{}", e))?;
    Ok(format!("已还原 {} 台显示器的分辨率、方向、缩放和桌面位置", saved.screens.len()))
}

fn mode_height(mode: &ScreenMode) -> u32 {
    // displayplacer lists modes for the display's current rotation, so res:h is
    // already the desktop's vertical axis. Swapping again double-rotates portrait modes.
    mode.h
}

fn mode_matches(actual: &Screen, wanted: &ScreenMode, degree: u32) -> bool {
    actual.active_mode().map(|m| (m.index, m.w, m.h, m.hz, m.depth, m.scaling))
        == Some((wanted.index, wanted.w, wanted.h, wanted.hz, wanted.depth, wanted.scaling))
        && actual.degree == degree
}

/// 体检 Mac 当前两屏模式与共同逻辑高度候选；EDID 物理尺寸因显示器/系统而异，此处不伪报物理高度。
pub fn align_report() -> Result<String, String> {
    let screens = parse_screens();
    if screens.len() < 2 {
        return Err("双屏对齐需要两块已连接显示器；请确认已安装 displayplacer".into());
    }
    let (main, sec) = main_secondary(&screens)?;
    if main.modes.is_empty() || sec.modes.is_empty() {
        return Err("displayplacer 没有提供可用模式列表；请更新 displayplacer 后重试".into());
    }
    let common: std::collections::BTreeSet<u32> = main
        .modes
        .iter()
        .flat_map(|a| sec.modes.iter().filter(move |b| mode_height(a) == mode_height(b)))
        .map(mode_height)
        .collect();
    let mut out = format!(
        "主屏当前：{} {}x{}@{}，方向 {}°；副屏当前：{} {}x{}@{}，方向 {}°。\n",
        main.id, main.frame_w(), main.frame_h(), hz_label(main.hz), main.degree,
        sec.id, sec.frame_w(), sec.frame_h(), hz_label(sec.hz), sec.degree
    );
    if common.is_empty() {
        out.push_str("两屏当前列出的模式没有相同逻辑高度。请检查系统显示设置是否提供共同高度模式。\n");
    } else {
        out.push_str(&format!("找到 {} 个共同逻辑高度：{}。\n", common.len(), common.iter().map(u32::to_string).collect::<Vec<_>>().join("、")));
        out.push_str("一键对齐会优先保留当前模式比例/刷新率，并把并排显示器的顶边对齐。逻辑高度相同不等于面板物理高度相同；实际物理边缘仍需目测确认。\n");
    }
    out.push_str("模式清单（主/副）：\n");
    out.push_str(&format!("主屏：{}\n", main.modes.iter().map(|m| format!("#{} {}x{}@{} {}bit {}", m.index, m.w, m.h, hz_label(m.hz), m.depth, if m.scaling { "scaled" } else { "native" })).collect::<Vec<_>>().join("；")));
    out.push_str(&format!("副屏：{}", sec.modes.iter().map(|m| format!("#{} {}x{}@{} {}bit {}", m.index, m.w, m.h, hz_label(m.hz), m.depth, if m.scaling { "scaled" } else { "native" })).collect::<Vec<_>>().join("；")));
    Ok(out)
}

/// 从两屏共同支持的模式中选择相同逻辑高度，备份全布局，应用并读取回实际状态；失败则回滚。
pub fn align_apply() -> Result<String, String> {
    let original = parse_screens();
    if original.len() < 2 {
        return Err("双屏对齐需要两块已连接显示器；请确认已安装 displayplacer".into());
    }
    let (main, sec) = main_secondary(&original)?;
    if main.modes.is_empty() || sec.modes.is_empty() {
        return Err("displayplacer 未列出两屏可用模式，无法安全对齐；请更新 displayplacer".into());
    }
    if main.active_mode().is_none() || sec.active_mode().is_none() {
        return Err("displayplacer 未能标记两屏当前模式，无法安全推算逻辑桌面尺寸；请更新 displayplacer 后重试".into());
    }
    let side_by_side = main.ox + main.frame_w() as i32 <= sec.ox || sec.ox + sec.frame_w() as i32 <= main.ox;
    if !side_by_side {
        return Err("当前两屏不是左右并排布局。请先在 macOS「显示器」设置中把两屏左右并排，再运行对齐".into());
    }
    let mut candidates: Vec<(f64, ScreenMode, ScreenMode, u32)> = Vec::new();
    for a in &main.modes {
        for b in &sec.modes {
            let ha = mode_height(a);
            let hb = mode_height(b);
            if ha != hb { continue; }
            let cur_a = main.active_mode().map(|m| (m.index, m.w, m.h, m.hz, m.depth, m.scaling))
                == Some((a.index, a.w, a.h, a.hz, a.depth, a.scaling));
            let cur_b = sec.active_mode().map(|m| (m.index, m.w, m.h, m.hz, m.depth, m.scaling))
                == Some((b.index, b.w, b.h, b.hz, b.depth, b.scaling));
            let aspect_delta = ((a.w as f64 / a.h as f64) - (main.frame_w() as f64 / main.frame_h().max(1) as f64)).abs()
                + ((b.w as f64 / b.h as f64) - (sec.frame_w() as f64 / sec.frame_h().max(1) as f64)).abs();
            let refresh_delta = option_delta(a.hz, main.hz) + option_delta(b.hz, sec.hz);
            let area = (a.w as f64 * a.h as f64 + b.w as f64 * b.h as f64).sqrt().max(1.0);
            let current_penalty = (if cur_a { 0.0 } else { 1.0 }) + (if cur_b { 0.0 } else { 1.0 });
            let score = current_penalty * 10000.0
                + aspect_delta * 100.0 + refresh_delta + 10000.0 / area;
            candidates.push((score, a.clone(), b.clone(), ha));
        }
    }
    candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if candidates.is_empty() {
        return Err("两块屏没有相同逻辑高度的共同显示模式，未更改布局".into());
    }
    save_original_layout(&original)?;
    let mut failures = Vec::new();
    for (_, main_mode, sec_mode, height) in candidates.iter().take(16) {
        let mut target = original.clone();
        let mi = target.iter().position(|s| s.id == main.id).ok_or("主屏已断开")?;
        let si = target.iter().position(|s| s.id == sec.id).ok_or("副屏已断开")?;
        target[mi].w = main_mode.w;
        target[mi].h = main_mode.h;
        target[mi].hz = main_mode.hz;
        target[mi].color_depth = main_mode.depth;
        target[mi].scaling = main_mode.scaling;
        target[mi].requested_mode = Some(main_mode.clone());
        target[si].w = sec_mode.w;
        target[si].h = sec_mode.h;
        target[si].hz = sec_mode.hz;
        target[si].color_depth = sec_mode.depth;
        target[si].scaling = sec_mode.scaling;
        target[si].requested_mode = Some(sec_mode.clone());
        // For side-by-side displays align top edges so the shared seam has no vertical offset.
        target[si].oy = target[mi].oy;
        if main.ox < sec.ox {
            target[si].ox = target[mi].ox + target[mi].frame_w() as i32;
        } else {
            target[si].ox = target[mi].ox - target[si].frame_w() as i32;
        }
        let applied = apply_screen_layout(&target);
        if applied.is_ok() {
            std::thread::sleep(std::time::Duration::from_millis(900));
            let actual = parse_screens();
            let got_main = actual.iter().find(|s| s.id == main.id);
            let got_sec = actual.iter().find(|s| s.id == sec.id);
            if let (Some(gm), Some(gs)) = (got_main, got_sec) {
                if mode_matches(gm, main_mode, main.degree)
                    && mode_matches(gs, sec_mode, sec.degree)
                    && mode_height(main_mode) == mode_height(sec_mode)
                    && gm.oy == gs.oy
                    && (gm.ox + gm.frame_w() as i32 == gs.ox || gs.ox + gs.frame_w() as i32 == gm.ox)
                {
                    return Ok(format!(
                        "已对齐 macOS 两屏逻辑高度：{}px。主屏 {}x{}@{}，副屏 {}x{}@{}；顶边已对齐。物理面板尺寸可能不同，请实际检查边缘。",
                        height, gm.frame_w(), gm.frame_h(), hz_label(gm.hz), gs.frame_w(), gs.frame_h(), hz_label(gs.hz)
                    ));
                }
            }
            failures.push(format!("{}px 模式应用后回读与目标不符", height));
        } else {
            failures.push(format!("{}px 模式应用失败：{}", height, applied.unwrap_err()));
        }
        if let Err(e) = apply_screen_layout(&original) {
            return Err(format!("对齐尝试失败，且自动回滚原布局也失败：{}。请在 macOS 显示器设置中恢复；备份保存在 ScreenGuard 应用支持目录。", e));
        }
        std::thread::sleep(std::time::Duration::from_millis(900));
        let rolled_back = parse_screens();
        let rollback_ok = original.iter().all(|wanted| {
            rolled_back.iter().any(|actual| {
                actual.id == wanted.id
                    && actual.enabled == wanted.enabled
                    && (!wanted.enabled || (
                        actual.w == wanted.w
                            && actual.h == wanted.h
                            && actual.hz == wanted.hz
                            && actual.color_depth == wanted.color_depth
                            && actual.scaling == wanted.scaling
                            && actual.ox == wanted.ox
                            && actual.oy == wanted.oy
                            && actual.degree == wanted.degree
                            && actual.main == wanted.main
                    ))
            })
        });
        if !rollback_ok {
            return Err("对齐失败后已发送回滚命令，但回读未能确认原始布局恢复；请在 macOS 显示器设置中检查，备份仍保留".into());
        }
    }
    clear_layout_backup()?;
    Err(format!("尝试共同高度失败：{}。已恢复原布局。", failures.join("；")))
}

// ===== Accessibility/System Events：列出与控制任意应用窗口 =====

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SavedWindow {
    token: i64,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    title: String,
}

fn listed_window_titles() -> &'static std::sync::Mutex<std::collections::HashMap<i64, String>> {
    static TITLES: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<i64, String>>> = std::sync::OnceLock::new();
    TITLES.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn window_backup_path() -> Result<std::path::PathBuf, String> {
    Ok(app_support_dir()?.join("window-span-backup.json"))
}

fn accessibility_error(raw: &str) -> String {
    let low = raw.to_ascii_lowercase();
    if low.contains("-25211") || low.contains("-1743") || low.contains("assistive access")
        || low.contains("not authorized") || low.contains("accessibility")
    {
        format!("macOS 尚未授予窗口控制权限。请到「系统设置 → 隐私与安全性 → 辅助功能」允许 ScreenGuard；若系统询问自动化控制，也请允许它控制 System Events。系统返回：{}", raw.trim())
    } else {
        raw.trim().to_string()
    }
}

fn run_system_events(script: &str) -> Result<String, String> {
    let out = run_cmd("osascript", &["-e", script]).map_err(|e| accessibility_error(&e))?;
    let out = out.trim().to_string();
    if let Some(err) = out.strip_prefix("ERR|") {
        return Err(accessibility_error(err));
    }
    Ok(out)
}

fn window_list_raw() -> Result<String, String> {
    let script = format!(r#"
tell application "System Events"
  set outText to ""
  set permissionError to ""
  repeat with p in application processes
    try
      if (background only of p) is false then
        set appName to name of p as text
        set appPid to unix id of p as integer
        if appPid is not {} and appName is not "Screenguard" and appName is not "ScreenGuard" and appName is not "screenguard" then
          repeat with wi from 1 to (count of windows of p)
            try
              set wnd to window wi of p
              set {{px, py}} to position of wnd
              set {{ww, wh}} to size of wnd
              set titleText to ""
              try
                set titleText to name of wnd as text
              end try
              set AppleScript's text item delimiters to "|"
              set titleText to (text items of (appName & " — " & titleText)) as text
              set AppleScript's text item delimiters to " "
              set titleText to titleText as text
              set AppleScript's text item delimiters to ""
              set token to appPid * 65536 + wi
              set outText to outText & "W|" & token & "|" & ww & "x" & wh & "|" & titleText & linefeed
            on error errMsg number errNum
              if errNum is -25211 or errNum is -1743 then set permissionError to (errNum as text) & " " & errMsg
            end try
          end repeat
        end if
      end if
    on error errMsg number errNum
      if errNum is -25211 or errNum is -1743 then set permissionError to (errNum as text) & " " & errMsg
    end try
  end repeat
  if permissionError is not "" then return "ERR|" & permissionError
  return outText
end tell
"#, std::process::id());
    run_system_events(&script)
}

fn decode_window_token(token: i64) -> Result<(i64, i64), String> {
    if token <= 0 { return Err("无效的窗口编号".into()); }
    let pid = token / 65536;
    let index = token % 65536;
    if pid <= 0 || index <= 0 { return Err("窗口编号已过期，请刷新窗口列表".into()); }
    Ok((pid, index))
}

fn read_window(token: i64) -> Result<SavedWindow, String> {
    let (pid, index) = decode_window_token(token)?;
    let script = format!(r#"
tell application "System Events"
  try
    set p to first application process whose unix id is {}
    set wnd to window {} of p
    set appName to name of p as text
    set {{px, py}} to position of wnd
    set {{ww, wh}} to size of wnd
    set titleText to ""
    try
      set titleText to name of wnd as text
    end try
    set titleText to appName & " — " & titleText
    return "OK|" & px & "|" & py & "|" & ww & "|" & wh & "|" & titleText
  on error errMsg number errNum
    return "ERR|" & errNum & " " & errMsg
  end try
end tell
"#, pid, index);
    let raw = run_system_events(&script)?;
    let p: Vec<&str> = raw.splitn(7, '|').collect();
    if p.len() < 6 || p[0] != "OK" {
        return Err(format!("窗口已关闭或 macOS 限制调整：{}", raw));
    }
    Ok(SavedWindow {
        token,
        x: p[1].parse().map_err(|_| "读取窗口横坐标失败")?,
        y: p[2].parse().map_err(|_| "读取窗口纵坐标失败")?,
        w: p[3].parse().map_err(|_| "读取窗口宽度失败")?,
        h: p[4].parse().map_err(|_| "读取窗口高度失败")?,
        title: p[5].to_string(),
    })
}

fn set_window_rect(token: i64, x: i32, y: i32, w: i32, h: i32) -> Result<SavedWindow, String> {
    let (pid, index) = decode_window_token(token)?;
    if w <= 0 || h <= 0 { return Err("目标窗口尺寸无效".into()); }
    let script = format!(r#"
tell application "System Events"
  try
    set p to first application process whose unix id is {}
    set wnd to window {} of p
    set appName to name of p as text
    set size of wnd to {{{}, {}}}
    set position of wnd to {{{}, {}}}
    delay 0.25
    set {{px, py}} to position of wnd
    set {{ww, wh}} to size of wnd
    set titleText to ""
    try
      set titleText to name of wnd as text
    end try
    set titleText to appName & " — " & titleText
    return "OK|" & px & "|" & py & "|" & ww & "|" & wh & "|" & titleText
  on error errMsg number errNum
    return "ERR|" & errNum & " " & errMsg
  end try
end tell
"#, pid, index, w, h, x, y);
    let raw = run_system_events(&script)?;
    let p: Vec<&str> = raw.splitn(7, '|').collect();
    if p.len() < 6 || p[0] != "OK" {
        return Err(format!("窗口拒绝调整大小：{}", raw));
    }
    Ok(SavedWindow {
        token,
        x: p[1].parse().map_err(|_| "读取窗口横坐标失败")?,
        y: p[2].parse().map_err(|_| "读取窗口纵坐标失败")?,
        w: p[3].parse().map_err(|_| "读取窗口宽度失败")?,
        h: p[4].parse().map_err(|_| "读取窗口高度失败")?,
        title: p[5].to_string(),
    })
}

fn save_window_backup(window: &SavedWindow) -> Result<(), String> {
    let path = window_backup_path()?;
    if path.exists() {
        let existing = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<SavedWindow>(&b).ok());
        if existing.as_ref().map(|w| w.token) == Some(window.token) { return Ok(()); }
    }
    save_json_atomic(&path, window)
}

fn union_or_primary(mode: u32) -> Result<(i32, i32, i32, i32), String> {
    let screens = parse_screens();
    if screens.iter().filter(|s| s.enabled).count() < 2 {
        return Err("双屏铺满需要至少两块已启用显示器".into());
    }
    if mode == 1 {
        if screens.iter().any(|s| s.enabled && s.layout_mode().is_none()) {
            return Err("无法识别当前显示模式的桌面尺寸；请更新 displayplacer 后重试，未调整窗口".into());
        }
        let active: Vec<&Screen> = screens.iter().filter(|s| s.enabled).collect();
        let main = active.iter().copied().find(|s| s.main).ok_or("未找到主屏")?;
        let min_x = active.iter().map(|s| s.ox).min().ok_or("无法计算桌面左边界")?;
        let max_x = active.iter().map(|s| s.ox + s.frame_w() as i32).max().ok_or("无法计算桌面右边界")?;
        // Match Windows mode=1: span the full horizontal desktop, using the main
        // display's height and top edge. This still covers both screens' widths.
        return Ok((min_x, main.oy, max_x, main.oy + main.frame_h() as i32));
    }
    union_bounds(&screens).ok_or_else(|| "无法计算屏幕布局边界".into())
}

pub fn set_screen_inches(_manuf: &str, _inches: f64) -> Result<(), String> {
    Err("macOS 版对齐依据两屏共同逻辑高度，不使用 Windows 的厂商代码尺寸录入".into())
}

fn restore_window_rect_after_failed_span(token: i64, original: &SavedWindow) -> String {
    match set_window_rect(token, original.x, original.y, original.w, original.h) {
        Ok(actual)
            if actual.title == original.title
                && (actual.x - original.x).abs() <= 8
                && (actual.y - original.y).abs() <= 8
                && (actual.w - original.w).abs() <= 8
                && (actual.h - original.h).abs() <= 8 =>
        {
            "窗口已自动恢复到铺满前的矩形".into()
        }
        Ok(actual) => format!(
            "自动回滚未完全恢复：原矩形 ({},{}) {}x{}，回读 ({},{}) {}x{}；窗口还原备份仍保留",
            original.x, original.y, original.w, original.h,
            actual.x, actual.y, actual.w, actual.h
        ),
        Err(e) => format!("自动回滚窗口失败：{}；窗口还原备份仍保留", e),
    }
}

fn span_to_rect(token: i64, mode: u32, expected_title: Option<&str>) -> Result<String, String> {
    let (x0, y0, x1, y1) = union_or_primary(mode)?;
    let original = read_window(token)?;
    if expected_title.is_some_and(|title| title != original.title) {
        return Err("窗口列表在操作期间已变化，未调整该编号当前指向的窗口；请刷新列表后重试".into());
    }
    let actual = match set_window_rect(token, x0, y0, x1 - x0, y1 - y0) {
        Ok(actual) => actual,
        Err(e) => {
            let rollback = restore_window_rect_after_failed_span(token, &original);
            return Err(format!("{}；{}", e, rollback));
        }
    };
    let dx = (actual.x - x0).abs();
    let dy = (actual.y - y0).abs();
    let dw = (actual.w - (x1 - x0)).abs();
    let dh = (actual.h - (y1 - y0)).abs();
    if expected_title.is_some_and(|title| title != actual.title)
        || dx > 8 || dy > 8 || dw > 8 || dh > 8
    {
        let reason = format!(
            "macOS/应用未接受跨屏窗口尺寸或窗口身份发生变化。目标边界 ({},{}) {}x{}，实际 ({},{}) {}x{}。请确认辅助功能授权；应用最小尺寸或 macOS 原生全屏 Space 也可能限制跨屏铺满。",
            x0, y0, x1-x0, y1-y0, actual.x, actual.y, actual.w, actual.h
        );
        let rollback = restore_window_rect_after_failed_span(token, &original);
        return Err(format!("{}；{}", reason, rollback));
    }
    Ok(actual.title)
}

pub fn list_windows() -> Result<String, String> {
    let raw = window_list_raw()?;
    let rows: Vec<&str> = raw.lines().filter(|l| l.starts_with("W|")).collect();
    let mut titles = listed_window_titles().lock().map_err(|_| "窗口列表状态不可用")?;
    titles.clear();
    for row in &rows {
        let fields: Vec<&str> = row.splitn(4, '|').collect();
        if fields.len() == 4 {
            if let Ok(token) = fields[1].parse::<i64>() {
                titles.insert(token, fields[3].to_string());
            }
        }
    }
    Ok(rows.join("\n"))
}

pub fn span_window(token: i64, mode: u32) -> Result<String, String> {
    let expected_title = listed_window_titles().lock()
        .map_err(|_| "窗口列表状态不可用")?
        .get(&token)
        .cloned()
        .ok_or("窗口编号已过期，请刷新窗口列表后重试")?;
    let before = read_window(token)?;
    if before.title != expected_title {
        return Err("窗口列表已变化，未调整其他窗口；请刷新窗口列表后重试".into());
    }
    // Capture the user's pre-alignment frame before changing display modes. The
    // later span operation must not replace this with a post-alignment rectangle.
    save_window_backup(&before)?;
    let aligned = mode == 2;
    if aligned {
        align_apply()?;
        std::thread::sleep(std::time::Duration::from_millis(900));
    }
    let result = span_to_rect(token, if mode == 1 { 1 } else { 0 }, Some(&expected_title));
    match (aligned, result) {
        (true, Err(e)) => {
            let rollback = align_restore_modes()
                .map(|s| format!("显示布局已回滚：{}", s))
                .unwrap_or_else(|re| format!("显示布局回滚失败：{}", re));
            Err(format!("{}；{}", e, rollback))
        }
        (_, other) => other,
    }
}

pub fn span_foreground(mode: u32) -> Result<String, String> {
    // Do not guess when clicking the control made ScreenGuard the frontmost app. The explicit
    // list_windows/span_window picker is reliable in that case and avoids resizing an unrelated app.
    let script = format!(r#"
tell application "System Events"
  try
    set p to first application process whose frontmost is true
    if (unix id of p as integer) is not {} and (name of p as text) is not "Screenguard" and (name of p as text) is not "ScreenGuard" and (name of p as text) is not "screenguard" then
      set pidValue to unix id of p as integer
      if (count of windows of p) > 0 then return "T|" & (pidValue * 65536 + 1)
    end if
    return "SELF"
  on error errMsg number errNum
    return "ERR|" & errNum & " " & errMsg
  end try
end tell
"#, std::process::id());
    let active = run_system_events(&script)?;
    if active == "SELF" {
        return Err("ScreenGuard 当前在前台，无法可靠判断你原先选中的窗口。请在「窗口选择」中刷新列表、选中目标窗口后铺满。".into());
    }
    let chosen = active.strip_prefix("T|")
        .ok_or_else(|| format!("无法定位前台应用窗口：{}", active))?
        .parse::<i64>().map_err(|_| "无法读取前台窗口编号")?;
    let before = read_window(chosen)?;
    save_window_backup(&before)?;
    let aligned = mode == 2;
    if aligned {
        align_apply()?;
        std::thread::sleep(std::time::Duration::from_millis(900));
    }
    let result = span_to_rect(chosen, if mode == 1 { 1 } else { 0 }, Some(&before.title));
    match (aligned, result) {
        (true, Err(e)) => {
            let rollback = align_restore_modes()
                .map(|s| format!("显示布局已回滚：{}", s))
                .unwrap_or_else(|re| format!("显示布局回滚失败：{}", re));
            Err(format!("{}；{}", e, rollback))
        }
        (_, other) => other,
    }
}

pub fn restore_foreground() -> Result<(), String> {
    let path = window_backup_path()?;
    if !path.exists() { return Ok(()); }
    let bytes = std::fs::read(&path).map_err(|e| format!("读取窗口还原信息失败：{}", e))?;
    let saved: SavedWindow = serde_json::from_slice(&bytes).map_err(|e| format!("窗口还原信息损坏：{}", e))?;
    // Restore the original display topology first so the saved AX frame is again
    // interpreted in the same desktop coordinate space in which it was captured.
    align_restore_modes()?;
    let current = read_window(saved.token)?;
    if current.title != saved.title {
        return Err("原窗口编号已指向其他窗口，未恢复错误窗口；请重新打开原应用并刷新窗口列表".into());
    }
    let actual = set_window_rect(saved.token, saved.x, saved.y, saved.w, saved.h)?;
    if (actual.x - saved.x).abs() > 8 || (actual.y - saved.y).abs() > 8
        || (actual.w - saved.w).abs() > 8 || (actual.h - saved.h).abs() > 8
    {
        return Err(format!("窗口未能完全还原：原位置 ({},{}) {}x{}，当前 ({},{}) {}x{}；还原信息已保留", saved.x, saved.y, saved.w, saved.h, actual.x, actual.y, actual.w, actual.h));
    }
    std::fs::remove_file(path).map_err(|e| format!("窗口已还原，但清除状态失败：{}", e))
}

// ===== 视频跨屏（VLC）= 按当前真实布局铺满所有屏的并集 =====
pub fn span_video() -> Result<(), String> {
    let installed =
        which("vlc") || std::path::Path::new("/Applications/VLC.app/Contents/MacOS/VLC").exists();
    if !installed {
        return Err("未安装 VLC".into());
    }
    let screens = parse_screens();
    if screens.iter().filter(|s| s.enabled).count() < 2 {
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
        .find(|s| s.enabled && s.main)
        .ok_or("未找到主屏")?;
    if main.layout_mode().is_none() {
        return Err("无法识别主屏当前桌面尺寸；请更新 displayplacer 后重试，未调整 VLC 窗口".into());
    }
    let x0 = main.ox;
    let y0 = main.oy;
    let x1 = x0 + main.frame_w() as i32;
    let y1 = y0 + main.frame_h() as i32;
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
