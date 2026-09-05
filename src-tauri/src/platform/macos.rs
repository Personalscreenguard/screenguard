use super::{run_cmd, which, DisplayInfo};

const DISPLAYPLACER: &str = "/opt/homebrew/bin/displayplacer";
const BETTERDISPLAY: &str = "/Applications/BetterDisplay.app/Contents/MacOS/BetterDisplay";

pub fn platform_name() -> &'static str {
    "macos"
}

fn betterdisplay_available() -> bool {
    std::path::Path::new(BETTERDISPLAY).exists()
}

/// 解析 displayplacer list 输出，得到显示器列表
pub fn get_displays() -> Vec<DisplayInfo> {
    let raw = run_cmd(DISPLAYPLACER, &["list"]).unwrap_or_default();
    let mut result = Vec::new();
    let mut cur_id = String::new();
    let mut cur_name = String::new();
    let mut cur_res = String::new();
    let mut cur_hz: Option<u32> = None;
    let mut cur_main = false;
    let mut cur_ctx: Option<u32> = None;

    for line in raw.lines() {
        let t = line.trim();
        if let Some(v) = t.strip_prefix("Persistent screen id:") {
            if !cur_id.is_empty() {
                push_display(
                    &mut result,
                    &cur_id,
                    &cur_name,
                    &cur_res,
                    cur_hz,
                    cur_main,
                    cur_ctx,
                );
            }
            cur_id = v.trim().to_string();
            cur_name = String::new();
            cur_res = String::new();
            cur_hz = None;
            cur_main = false;
            cur_ctx = None;
        } else if let Some(v) = t.strip_prefix("Contextual screen id:") {
            cur_ctx = v.trim().parse::<u32>().ok();
        } else if let Some(v) = t.strip_prefix("Type:") {
            cur_name = decode_name(v.trim());
        } else if let Some(v) = t.strip_prefix("Resolution:") {
            cur_res = v.trim().to_string();
        } else if let Some(v) = t.strip_prefix("Hertz:") {
            cur_hz = v.trim().parse::<u32>().ok();
        } else if let Some(v) = t.strip_prefix("Origin:") {
            cur_main = v.contains("main");
        }
    }
    if !cur_id.is_empty() {
        push_display(
            &mut result,
            &cur_id,
            &cur_name,
            &cur_res,
            cur_hz,
            cur_main,
            cur_ctx,
        );
    }
    result
}

fn decode_name(name: &str) -> String {
    let n = name.to_string();
    if n.starts_with("27 inch") {
        "主屏（27寸）".to_string()
    } else if n.starts_with("31 inch") || n.starts_with("16 inch") {
        "副屏（16寸）".to_string()
    } else {
        n
    }
}

fn push_display(
    v: &mut Vec<DisplayInfo>,
    id: &str,
    name: &str,
    res: &str,
    hz: Option<u32>,
    main: bool,
    cgid: Option<u32>,
) {
    let (b, vol) = match cgid {
        Some(c) if betterdisplay_available() => (read_ddc_brightness(c), read_ddc_volume(c)),
        _ => (None, None),
    };
    let prof = cgid.and_then(read_color_profile);
    v.push(DisplayInfo {
        id: id.to_string(),
        name: name.to_string(),
        resolution: res.to_string(),
        hz,
        main,
        connected: true,
        ddc: b.is_some(),
        brightness: b,
        volume: vol,
        color_profile: prof,
        ddc_id: cgid,
    });
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

/// 设置亮度 0-100
pub fn set_brightness(display_id: &str, value: u32) -> Result<(), String> {
    if !betterdisplay_available() {
        return Err("未检测到 BetterDisplay（请安装并启动）".into());
    }
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
    if !betterdisplay_available() {
        return Err("未检测到 BetterDisplay（请安装并启动）".into());
    }
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
    if !betterdisplay_available() {
        return Err("未检测到 BetterDisplay（请安装并启动）".into());
    }
    let profile = space_profile_url(space);
    let displays = get_displays();
    let mut applied = 0;
    for d in &displays {
        if let Some(cgid) = d.ddc_id {
            if run_cmd(
                BETTERDISPLAY,
                &[
                    "set",
                    &ddctl(cgid),
                    &format!("-colorProfileURL={}", profile),
                ],
            )
            .is_ok()
            {
                applied += 1;
            }
        }
    }
    if applied == 0 {
        Err("未能应用色彩配置".into())
    } else {
        Ok(())
    }
}

pub fn match_mac() -> Result<(), String> {
    // Apple 内建屏为 Display P3，将外接屏全部对齐到 Display P3（软件可落地的"对齐 Mac"）
    apply_color_space("p3")
}

pub fn match_ppi() -> Result<(), String> {
    let _ = run_cmd(
        DISPLAYPLACER,
        &[
            "id:5627366A-C9B8-4D46-ABDF-00FA9C4A9576",
            "res:1920x1080",
            "id:14A34586-7834-4EFF-BEFE-E014A08A4BEA",
            "res:1920x1080",
        ],
    );
    Ok(())
}

const SEC_ID: &str = "14A34586-7834-4EFF-BEFE-E014A08A4BEA";
const MAIN_ID: &str = "5627366A-C9B8-4D46-ABDF-00FA9C4A9576";

/// 读副屏当前旋转角度（0=横屏，270=竖屏）
fn secondary_degree() -> u32 {
    let raw = run_cmd(DISPLAYPLACER, &["list"]).unwrap_or_default();
    let mut in_sec = false;
    for line in raw.lines() {
        let t = line.trim();
        if t.starts_with("Persistent screen id:") {
            in_sec = t.contains(SEC_ID);
        } else if in_sec {
            if let Some(v) = t.strip_prefix("Rotation:") {
                return v.trim().parse().unwrap_or(0);
            }
        }
    }
    0
}

/// 用已知模式设置副屏（避免不支持的组合导致错乱）
fn set_secondary(res: &str, degree: u32) -> Result<(), String> {
    // 明确指定两块屏，副屏用已验证的模式切换
    let main = format!(
        "id:{} res:1920x1080 hz:60 color_depth:8 enabled:true scaling:on origin:(0,0) degree:0",
        MAIN_ID
    );
    let sec = format!(
        "id:{} res:{} hz:60 color_depth:8 enabled:true scaling:on origin:(1920,0) degree:{}",
        SEC_ID, res, degree
    );
    run_cmd(DISPLAYPLACER, &[&main, &sec]).map(|_| ())
}

/// 副屏横/竖屏切换（再点一次转回，可恢复）
pub fn rotate_secondary() -> Result<(), String> {
    if secondary_degree() == 0 {
        set_secondary("1080x1920", 270) // 横屏 -> 竖屏
    } else {
        set_secondary("1920x1080", 0) // 竖屏 -> 横屏
    }
}

/// 恢复副屏为竖屏（安全复位）
pub fn restore_secondary() -> Result<(), String> {
    set_secondary("1080x1920", 270)
}

// ===== 视频跨屏（VLC）= 按当前布局铺满两块屏的并集 =====
pub fn span_video() -> Result<(), String> {
    let installed = which("vlc")
        || std::path::Path::new("/Applications/VLC.app/Contents/MacOS/VLC").exists();
    if !installed {
        return Err("未安装 VLC".into());
    }

    // 不再自动改副屏方向，保留用户现有布局；
    // 按当前两块屏的并集来铺满（副屏横屏=3840x1080，竖屏=3000x1920）
    let (w, h) = if secondary_degree() == 0 {
        (3840u32, 1080u32)
    } else {
        (3000u32, 1920u32)
    };

    // 用 osascript 统一处理：activate 启动 VLC（走 LaunchServices，AppleScript 端点一致）→ 等主窗口出现 → 铺满
    let script = format!(
        r#"
on run
  tell application "VLC" to activate
  delay 2.0
  set w to {}
  set h to {}
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
          set bounds of targetWindow to {{0, 0, w, h}}
          delay 0.4
        end if
      end try
    end repeat
  end tell
end run
"#,
        w, h
    );
    run_cmd("osascript", &["-e", &script])?;
    Ok(())
}

pub fn restore_video() -> Result<(), String> {
    // 把 VLC 主窗口恢复为铺满主屏（退出跨屏状态）
    let script = r#"
on run
  tell application "VLC" to activate
  delay 1.0
  set w to 1920
  set h to 1080
  tell application "VLC"
    set targetWindow to missing value
    set maxArea to 0
    repeat 10 times
      try
        set targetWindow to missing value
        set maxArea to 0
        repeat with wnd in windows
          try
            set {x1, y1, x2, y2} to bounds of wnd
            set area to (x2 - x1) * (y2 - y1)
            if area > maxArea then
              set maxArea to area
              set targetWindow to wnd
            end if
          end try
        end repeat
        if targetWindow is not missing value then
          set bounds of targetWindow to {0, 0, w, h}
          delay 0.4
        end if
      end try
    end repeat
  end tell
end run
"#;
    run_cmd("osascript", &["-e", script])?;
    Ok(())
}
