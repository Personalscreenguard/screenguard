// 屏幕守护 — 跨平台前端逻辑 (Tauri 2)
// 通过 window.__TAURI__.core.invoke 调用 Rust 后端命令

const statusEl = document.getElementById('status');
const platformBadge = document.getElementById('platform-badge');

function setStatus(msg, ok = true) {
  statusEl.textContent = msg;
  statusEl.style.color = ok ? '' : '#f0706f';
}

function setBadge(txt) { platformBadge.textContent = txt; }

// 按钮常带图标：只改文字节点，不覆盖图标
function setBtnLabel(id, text) {
  const el = document.getElementById(id);
  if (!el) return;
  const label = el.querySelector('.btn-label');
  (label || el).textContent = text;
}

function invoke(cmd, args = {}) {
  // Tauri 2：核心 API 暴露在 window.__TAURI__.core
  if (window.__TAURI__ && window.__TAURI__.core) {
    return window.__TAURI__.core.invoke(cmd, args);
  }
  // 调试/无 Tauri 环境：返回假数据，避免页面空白
  return Promise.reject(new Error('TAURI_CORE_UNAVAILABLE'));
}


// 后端 platform_name() 在 macOS 返回 "macOS"（大写 O），
// 统一归一成 'macos'/'windows'/'linux' 再做判断，避免大小写导致的平台分支失效。
function normPlatform(p) {
  const v = String(p || '').toLowerCase();
  if (v.startsWith('mac') || v === 'darwin') return 'macos';
  if (v.startsWith('win')) return 'windows';
  return v;
}
function getPlatform() { return invoke('get_platform').then(normPlatform); }

const RELEASES_PAGE = 'https://github.com/Personalscreenguard/screenguard/releases/latest';
const UPDATE_API = 'https://api.github.com/repos/Personalscreenguard/screenguard/releases/latest';
const UPDATE_CACHE_KEY = 'sg_update_check_v1';
function versionParts(version) {
  const match = String(version || '').trim().replace(/^v/i, '').match(/^(\d+)\.(\d+)\.(\d+)/);
  return match ? match.slice(1).map(Number) : null;
}
function isNewerVersion(candidate, current) {
  const a = versionParts(candidate), b = versionParts(current);
  if (!a || !b) return false;
  for (let i = 0; i < 3; i++) { if (a[i] !== b[i]) return a[i] > b[i]; }
  return false;
}
async function checkForUpdates(force = false, currentVersion = '', quiet = false) {
  const button = document.getElementById('update-check');
  if (!button) return;
  // 提示统一写底部状态栏；静默检查只在发现新版本时占版面
  const say = text => { if (!quiet) setStatus(text); };
  button.disabled = true;
  say('正在检查更新…');
  try {
    const cached = JSON.parse(localStorage.getItem(UPDATE_CACHE_KEY) || 'null');
    const fresh = cached && Date.now() - Number(cached.checkedAt) < 24 * 60 * 60 * 1000;
    let release = !force && fresh ? cached.release : null;
    if (!release) {
      const controller = new AbortController();
      const timeout = setTimeout(() => controller.abort(), 9000);
      try {
        const response = await fetch(UPDATE_API, {
          headers: { Accept: 'application/vnd.github+json' },
          cache: 'no-store',
          signal: controller.signal
        });
        if (!response.ok) throw new Error(`GitHub 返回 HTTP ${response.status}`);
        const data = await response.json();
        release = { tag: String(data.tag_name || ''), url: String(data.html_url || RELEASES_PAGE) };
        if (!versionParts(release.tag) || !/^https:\/\/github\.com\/Personalscreenguard\/screenguard\/releases\//.test(release.url)) {
          throw new Error('Release 信息格式无效');
        }
        localStorage.setItem(UPDATE_CACHE_KEY, JSON.stringify({ checkedAt: Date.now(), release }));
      } finally { clearTimeout(timeout); }
    }
    const version = currentVersion || await invoke('get_version');
    if (isNewerVersion(release.tag, version)) {
      // 静默检查也要让「发现新版本」可见
      setStatus(`发现新版本 ${release.tag}（当前 v${version}），点「打开下载页」前往`);
      button.textContent = '打开下载页';
      button.dataset.mode = 'download';
      button.disabled = false;
      return;
    }
    say(`已是最新版本 v${version}`);
  } catch (error) {
    say(`无法检查更新：${errText(error)}`);
  } finally { button.disabled = false; }
}
document.getElementById('update-check')?.addEventListener('click', async () => {
  const btn = document.getElementById('update-check');
  if (btn?.dataset.mode === 'download') {
    try { await invoke('open_external_page', { page: 'releases' }); }
    catch (error) { setStatus('无法打开下载页：' + errText(error), false); }
    return;
  }
  checkForUpdates(true);
});
document.getElementById('privacy-page')?.addEventListener('click', async () => {
  try { await invoke('open_external_page', { page: 'privacy' }); }
  catch (error) { setStatus(`无法打开隐私说明：${errText(error)}`, false); }
});

async function platformSettingsBackup() {
  const platform = await getPlatform();
  const version = await invoke('get_version');
  const native = JSON.parse(await invoke('export_user_settings_json') || '{}');
  const uiPreferences = {};
  for (const key of Object.keys(localStorage)) {
    if (key.startsWith('sg_') && key !== UPDATE_CACHE_KEY) uiPreferences[key] = localStorage.getItem(key);
  }
  return {
    format: 'screenguard-settings', schemaVersion: 1, platform,
    appVersion: version, exportedAt: new Date().toISOString(),
    preferences: {
      local: uiPreferences,
      native,
      powerLinkEnabled: await invoke('get_power_link_preference').catch(() => false)
    }
  };
}
document.getElementById('settings-export')?.addEventListener('click', async () => {
  const status = document.getElementById('settings-backup-status');
  try {
    const bundle = await platformSettingsBackup();
    const blob = new Blob([JSON.stringify(bundle, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = `Screenguard-settings-${bundle.platform}-${new Date().toISOString().slice(0, 10)}.json`;
    document.body.appendChild(link); link.click(); link.remove(); setTimeout(() => URL.revokeObjectURL(url), 1000);
    status.textContent = '设置备份已导出。文件可能含本地 IP 和设备标识，请妥善保管。';
  } catch (error) { status.textContent = `导出失败：${errText(error)}`; }
});
const settingsFile = document.getElementById('settings-file');
document.getElementById('settings-import')?.addEventListener('click', () => settingsFile?.click());
settingsFile?.addEventListener('change', async () => {
  const file = settingsFile.files && settingsFile.files[0];
  if (!file) return;
  const status = document.getElementById('settings-backup-status');
  try {
    if (file.size > 1024 * 1024) throw new Error('设置备份文件超过 1 MB');
    const bundle = JSON.parse(await file.text());
    const platform = await getPlatform();
    if (bundle.format !== 'screenguard-settings' || bundle.schemaVersion !== 1 || !bundle.preferences) {
      throw new Error('这不是受支持的 Screenguard 设置备份');
    }
    if (bundle.platform !== platform) throw new Error('备份来自另一操作系统；为避免导入不兼容的设备设置，请在相同平台恢复');
    const local = bundle.preferences.local;
    if (!local || typeof local !== 'object' || Array.isArray(local)) throw new Error('本地偏好格式无效');
    for (const [key, value] of Object.entries(local)) {
      if (!/^sg_[a-z0-9_]{1,100}$/i.test(key) || typeof value !== 'string' || value.length > 20000) {
        throw new Error('备份含无效的本地偏好');
      }
    }
    if (!confirm('导入会覆盖 Screenguard 当前的应用偏好。硬件恢复基线和日志不会被覆盖。继续吗？')) return;
    const nativeJson = JSON.stringify(bundle.preferences.native || {});
    const imported = await invoke('import_user_settings_json', { settingsJson: nativeJson });
    await invoke('set_power_link_preference', { enabled: bundle.preferences.powerLinkEnabled === true });
    for (const key of Object.keys(localStorage)) if (key.startsWith('sg_')) localStorage.removeItem(key);
    for (const [key, value] of Object.entries(local)) localStorage.setItem(key, value);
    await window.refreshPowerLinkButton?.();
    status.textContent = `${imported}。本地应用偏好已恢复；硬件基线仍保留本机数据。`;
  } catch (error) { status.textContent = `导入失败：${errText(error)}`; }
  finally { settingsFile.value = ''; }
});

// DDC 写入串行化 + 合并最新值：拖动滑块时最多只有 1 个后台进程在跑，
// 中间值被覆盖、最终值保证送达（此前每格都新起 powershell 进程，拖动即卡死）
const vpState = new Map();
function sendVCP(kind, id, val) {
  const key = kind + ':' + id;
  let st = vpState.get(key);
  if (!st) { st = { latest: null, busy: false }; vpState.set(key, st); }
  st.latest = val;
  if (st.busy) return;
  st.busy = true;
  (async () => {
    while (st.latest !== null) {
      const v = st.latest; st.latest = null;
      const cmd = kind === 'brightness' ? 'set_brightness' : 'set_volume';
      try { await invoke(cmd, { displayId: id, value: Number(v) }); } catch (e) { /* 忽略瞬时失败 */ }
    }
    st.busy = false;
  })();
}

// ===== 屏幕电源（纯本地，不依赖网络）=====
// 全部待机走平台系统级指令：一次控制全部屏，含不走 DDC 的屏。
// 操作结果统一进底部状态栏，卡片里不再堆一行行小字说明。
function pwStat(t, ok = true) { setStatus(t, ok); }

document.getElementById('pw-off').addEventListener('click', async () => {
  pwStat('正在让全部屏幕待机…');
  try { await invoke('screen_off'); pwStat('已待机（动一下鼠标或按键盘即可唤醒）'); }
  catch (e) { pwStat('待机失败：' + (e && e.message ? e.message : e), false); }
});

document.getElementById('pw-wake').addEventListener('click', async () => {
  try { await invoke('screen_wake'); pwStat('已唤醒屏幕'); }
  catch (e) { pwStat('唤醒失败：' + (e && e.message ? e.message : e), false); }
});

// 主窗口也提供电源联动开关，避免用户只看到待机/唤醒按钮而不知道还需启用联动。
const POWER_LINK_KEY = 'sg_power_link_enabled';
const powerLinkBtn = document.getElementById('power-link-btn');
async function readPowerLinkPreference() {
  try {
    const enabled = Boolean(await invoke('get_power_link_preference'));
    localStorage.setItem(POWER_LINK_KEY, enabled ? '1' : '0');
    return enabled;
  } catch (_) {
    return localStorage.getItem(POWER_LINK_KEY) === '1';
  }
}
function paintPowerLinkButton(enabled) {
  const label = powerLinkBtn.querySelector('.switch-text');
  if (label) label.textContent = enabled ? '已开启' : '关闭';
  powerLinkBtn.classList.toggle('is-on', enabled);
  powerLinkBtn.setAttribute('aria-pressed', String(enabled));
  powerLinkBtn.title = enabled
    ? '已开启。后台监测显示器电源与连接变化；macOS 还监测 Quartz 单屏睡眠状态。'
    : '开启后监测显示器电源、Quartz 睡眠与连接变化；若物理关屏不改变任何系统可见状态，macOS 无法检测。';
}
async function refreshPowerLinkButton() {
  paintPowerLinkButton(await readPowerLinkPreference());
}
window.applyPowerLinkUiState = enabled => {
  try { localStorage.setItem(POWER_LINK_KEY, enabled ? '1' : '0'); } catch (_) {}
  paintPowerLinkButton(Boolean(enabled));
  pwStat(enabled
    ? '电源联动已开启；保持 Screenguard 运行，后台会同步显示器电源变化。'
    : '电源联动已关闭。');
};
if (window.__TAURI__ && window.__TAURI__.event) {
  window.__TAURI__.event.listen('power-link-status', event => {
    const message = String(event.payload || '');
    if (message) pwStat(message, !/失败|未完成|超时/.test(message));
  }).catch(() => {});
}
refreshPowerLinkButton();
window.refreshPowerLinkButton = refreshPowerLinkButton;
document.getElementById('open-panel-btn').addEventListener('click', async () => {
  try {
    const powerLinkEnabled = await readPowerLinkPreference();
    await invoke('open_control_panel', { powerLinkEnabled });
    setStatus('快捷控制面板已打开；电源联动状态未更改');
  } catch (error) {
    setStatus('打开快捷面板失败：' + errText(error), false);
  }
});
window.addEventListener('focus', refreshPowerLinkButton);
window.addEventListener('storage', ev => {
  if (ev.key === POWER_LINK_KEY) refreshPowerLinkButton();
});
powerLinkBtn.addEventListener('click', async () => {
  const previous = await readPowerLinkPreference();
  const enabled = !previous;
  try {
    await invoke('set_power_link_preference', { enabled });
    localStorage.setItem(POWER_LINK_KEY, enabled ? '1' : '0');
    await refreshPowerLinkButton();
    await invoke('open_control_panel', { powerLinkEnabled:enabled });
    pwStat(enabled
      ? '电源联动已开启；保持 Screenguard 运行，后台会同步显示器电源变化。'
      : '电源联动已关闭。');
  } catch (e) {
    try { await invoke('set_power_link_preference', { enabled: previous }); } catch (_) {}
    localStorage.setItem(POWER_LINK_KEY, previous ? '1' : '0');
    await refreshPowerLinkButton();
    pwStat('无法打开控制面板：' + (e && e.message ? e.message : e), false);
  }
});

// 连点「刷新」或切换模式后自动刷新时，只认最后一次请求的结果，
// 避免先发后到的旧数据覆盖新状态。
let displaysRequestId = 0;
async function refreshDisplays() {
  const requestId = ++displaysRequestId;
  try {
    setStatus('读取显示器…');
    const displays = await invoke('get_displays');
    if (requestId !== displaysRequestId) return;
    renderDisplays(displays);
    loadNativeDisplayTargets(displays);
    setStatus('就绪');
  } catch (e) {
    if (requestId !== displaysRequestId) return;
    renderDisplays([]);
    setStatus(e.message === 'TAURI_CORE_UNAVAILABLE' ? '非 Tauri 环境（预览模式）' : '读取显示器失败：' + e.message, false);
  }
}

function loadNativeDisplayTargets(displays) {
  const card = document.getElementById('native-display-card');
  const source = document.getElementById('nd-source');
  const target = document.getElementById('nd-target');
  if (!card || !source || !target || card.style.display === 'none') return;
  const previousSource = source.value;
  const previousTarget = target.value;
  const options = (displays || []).map(d => ({
    id: d.id,
    label: `${d.name || d.id}${d.main && !String(d.name || '').includes('主屏') ? '（主屏）' : ''} · ${d.resolution || ''}`
  }));
  for (const select of [source, target]) {
    select.replaceChildren();
    if (!options.length) {
      const empty = document.createElement('option');
      empty.value = '';
      empty.textContent = '未检测到显示器';
      select.appendChild(empty);
    } else {
      for (const item of options) {
        const option = document.createElement('option');
        option.value = item.id;
        option.textContent = item.label;
        select.appendChild(option);
      }
    }
  }
  if (options.some(d => d.id === previousSource)) source.value = previousSource;
  else source.value = (displays || []).find(d => d.main)?.id || options[0]?.id || '';
  if (options.some(d => d.id === previousTarget)) target.value = previousTarget;
  else target.value = options.find(d => d.id !== source.value)?.id || '';
  for (const button of card.querySelectorAll('[data-native-action]')) {
    const action = button.dataset.nativeAction || '';
    button.disabled = !options.length || (action.startsWith('place-') || action === 'mirror-on') && options.length < 2;
  }
}

document.querySelectorAll('#native-display-card [data-native-action]').forEach(button => {
  button.addEventListener('click', async () => {
    const source = document.getElementById('nd-source');
    const target = document.getElementById('nd-target');
    const action = button.dataset.nativeAction;
    const input = document.getElementById('nd-input-code');
    button.disabled = true;
    setStatus('正在执行…');
    try {
      const result = await invoke('display_action', {
        action,
        displayId: source.value,
        targetId: target.value || null,
        value: action === 'input-source' ? input.value.trim() : null
      });
      setStatus(String(result || '显示设置已更新'));
      await refreshDisplays();
    } catch (error) {
      setStatus('高级显示控制失败：' + errText(error), false);
    } finally {
      button.disabled = false;
      loadNativeDisplayTargets(await invoke('get_displays').catch(() => []));
    }
  });
});

function renderDisplays(displays) {
  const box = document.getElementById('displays');
  if (!displays || displays.length === 0) {
    box.innerHTML = '<div class="empty">未检测到显示器：请确认显示器已连接，并检查依赖是否就绪</div>';
    return;
  }
  box.innerHTML = ''; // 清空 + 重建（保留简洁结构）

  // 驱动栈异常启发式：仅一块屏且模式是 1024x768@60（本机 NVIDIA 驱动崩溃循环时实测形态）
  const degraded = displays.length === 1 &&
    displays[0].resolution === '1024x768' && displays[0].hz === 60;

  displays.forEach(d => {
    const row = document.createElement('div');
    row.className = 'display-row';
    row.dataset.displayId = d.id;

    const hasBri = d.ddc || d.brightness != null; // DDC 或 WMI（内屏）亮度
    const briBadge = d.ddc ? '<span class="badge ddc">DDC</span>'
      : (hasBri ? '<span class="badge ddc">WMI</span>' : '');
    const connectedBadge = d.connected
      ? '<span class="badge on">已连接</span>'
      : '<span class="badge off">未连接</span>';

    const briRow = hasBri ? `
      <div class="slider-row">
        <label>亮度</label>
        <input type="range" min="0" max="100" value="${d.brightness ?? 50}" data-display="${d.id}" data-kind="brightness">
        <span class="val">${d.brightness ?? 50}</span>
      </div>` : '';
    // 音量滑块 = DDC 0x62，只对带喇叭的显示器有意义（笔记本喇叭/耳机走「系统音量」）
    const volRow = d.ddc ? `
      <div class="slider-row">
        <label>音量</label>
        <input type="range" min="0" max="100" value="${d.volume ?? 50}" data-display="${d.id}" data-kind="volume">
        <span class="val">${d.volume ?? 50}</span>
      </div>` : '';
    // 只有「驱动栈可能异常」这类需要用户立刻处理的情况才提示，其余说明一律不占版面。
    const degradeAlert = degraded ? `
      <div class="alert">仅一块屏且运行在 1024x768@60，显示驱动栈可能异常。按 Win+Ctrl+Shift+B 重启图形驱动，画面恢复后点「刷新」。</div>` : '';

    row.innerHTML = `
      <div class="display-head">
        <div>
          <div class="display-name">${d.name || d.id}</div>
          <div class="display-meta">${d.resolution || ''} · ${d.hz ? d.hz + 'Hz' : ''}${d.color_profile ? ' · ' + d.color_profile : ''}</div>
        </div>
        <div class="badges">${briBadge}${connectedBadge}</div>
      </div>
      ${briRow}${volRow}${degradeAlert}
    `;
    box.appendChild(row);
  });

  // 绑定滑块
  box.querySelectorAll('input[type=range]').forEach(sl => {
    sl.addEventListener('input', (ev) => {
      const valEl = ev.target.parentElement.querySelector('.val');
      if (valEl) valEl.textContent = ev.target.value;
      const kind = ev.target.dataset.kind;
      sendVCP(kind, ev.target.dataset.display, ev.target.value);
      setStatus(`${kind === 'brightness' ? '亮度' : '音量'} → ${ev.target.value}`);
    });
  });
  void loadDisplayModes(displays, box);
}

async function loadDisplayModes(displays, box) {
  for (const display of displays) {
    try {
      const modes = await invoke('get_display_modes', { displayId: display.id });
      if (!Array.isArray(modes) || modes.length === 0) continue;
      const row = [...box.querySelectorAll('.display-row')]
        .find(element => element.dataset.displayId === display.id);
      if (!row) continue;
      const control = document.createElement('div');
      control.className = 'slider-row mode-row';
      const label = document.createElement('label');
      label.textContent = '分辨率';
      const select = document.createElement('select');
      select.className = 'display-mode-select';
      select.title = '切换时画面可能短暂黑屏；系统未应用时会自动恢复原设置';
      let activeMode = null;
      for (const mode of modes) {
        const option = document.createElement('option');
        option.value = String(mode.mode);
        option.textContent = `${mode.resolution} · ${mode.hz}Hz${mode.scaled ? ' · 缩放' : ''}`;
        if (mode.current) {
          option.selected = true;
          activeMode = String(mode.mode);
        }
        select.appendChild(option);
      }
      const state = document.createElement('span');
      state.className = 'val';
      control.append(label, select, state);
      row.appendChild(control);
      select.addEventListener('change', async () => {
        const selectedMode = Number(select.value);
        select.disabled = true;
        state.textContent = '切换中…';
        setStatus('正在应用显示模式…');
        try {
          const result = await invoke('set_display_mode', {
            displayId: display.id,
            mode: selectedMode,
          });
          activeMode = String(selectedMode);
          state.textContent = '已核验';
          setStatus(result || '显示模式已切换');
          await new Promise(resolve => setTimeout(resolve, 800));
          await refreshDisplays();
        } catch (error) {
          if (activeMode !== null) select.value = activeMode;
          state.textContent = '未切换';
          setStatus('显示模式切换失败：' + errText(error), false);
        } finally {
          select.disabled = false;
        }
      });
    } catch (_) {
      // Windows/Linux 当前由系统设置或各自平台工具管理显示模式。
    }
  }
}

// ===== 系统音量（Windows：默认输出端点，托盘音量同源） =====
// 拖动串行化与 DDC 滑块同一套策略：最多 1 个后台 powershell，合并最新值
const sysVol = { latest: null, busy: false, muted: false };
function sendSysVol(val) {
  sysVol.latest = val;
  if (sysVol.busy) return;
  sysVol.busy = true;
  (async () => {
    while (sysVol.latest !== null) {
      const v = sysVol.latest; sysVol.latest = null;
      try { await invoke('set_system_volume', { value: Number(v) }); } catch (e) { setStatus('音量设置失败：' + errText(e), false); }
    }
    sysVol.busy = false;
  })();
}

let audioRequestId = 0;
async function loadAudio() {
  const requestId = ++audioRequestId;
  const card = document.getElementById('audio-card');
  const devEl = document.getElementById('audio-dev');
  const sl = document.getElementById('sys-vol');
  const val = document.getElementById('sys-vol-val');
  const muteBtn = document.getElementById('mute-btn');
  const sel = document.getElementById('audio-dev-sel');
  try {
    const platform = await getPlatform();
    card.style.display = '';
    if (platform === 'macos') {
      if (sel) sel.closest('.field').style.display = 'none';
      document.getElementById('all-bri').closest('.slider-row').style.display = 'none';
      document.getElementById('mitv-row').style.display = 'none';
      const resetBtn = document.getElementById('bri-reset');
      resetBtn.style.display = 'none';
      resetBtn.parentElement.classList.add('solo');
    }
    // 音量作用于「用户选定的输出端点」。默认输出若正好是固定音量的
    // HDMI/DP 显示器音频（Windows 自己的滑块也调不动），就自动落在一个可调的端点上。
    const eps = await invoke('audio_endpoints');
    let st = {};
    try { st = JSON.parse(await invoke('app_state_json') || '{}'); } catch (e) {}
    const wanted = eps.find(e => e.id === st.audio_dev);
    const cur = wanted || eps.find(e => e.is_default) || eps.find(e => e.adjustable) || eps[0];
    if (requestId !== audioRequestId) return;
    if (sel) {
      sel.innerHTML = eps.map(e =>
        `<option value="${e.id}"${(cur && e.id === cur.id) ? ' selected' : ''}>${e.name}　${e.adjustable ? ('音量 ' + e.volume + '%') : '（固定音量·不可调）'}</option>`
      ).join('');
    }
    if (!cur) { devEl.textContent = '没有可用的输出设备'; sl.disabled = true; return; }
    devEl.textContent = cur.name || '输出设备';
    sl.value = cur.volume;
    val.textContent = cur.adjustable ? (cur.volume + '%') : '不可调';
    sysVol.muted = cur.mute;
    muteBtn.textContent = cur.mute ? '取消静音' : '静音';
    sl.disabled = !cur.adjustable;
    muteBtn.disabled = !cur.adjustable;
  } catch (e) {
    card.style.display = '';
    devEl.textContent = '不可用';
    setStatus('读取输出设备失败：' + errText(e), false);
  }
}

// ===== 电池设备（Windows：Battery 设备类枚举，内电池 / USB·无线键鼠 / 蓝牙） =====
function battIcon(conn) {
  const name = conn === '内置' ? 'laptop'
    : conn === '蓝牙' ? 'headphone'
      : (conn === 'USB/无线' || conn === 'USB') ? 'mouse'
        : 'battery';
  return `<svg class="ic"><use href="#ic-${name}"/></svg>`;
}
function battState(charging, onAc, percent) {
  if (percent < 0) return { cls: 'unknown', txt: '读取不到' };
  if (charging) return { cls: 'charging', txt: '充电中' };
  if (onAc) return { cls: 'ac', txt: '已接通电源' };
  return { cls: 'idle', txt: '放电中' };
}
async function loadBatteries() {
  const card = document.getElementById('battery-card');
  const box = document.getElementById('batteries');
  try {
    const list = await invoke('get_batteries');
    card.style.display = '';
    if (!list || list.length === 0) {
      box.innerHTML = '<div class="empty">未检测到带电池的设备</div>';
      return;
    }
    box.innerHTML = '';
    list.forEach(b => {
      const st = battState(b.charging, b.on_ac, b.percent);
      const pct = b.percent >= 0 ? b.percent : null;
      const barCls = pct == null ? '' : (b.charging ? ' charging' : (pct <= 20 ? ' low' : ''));
      const barWidth = pct == null ? 0 : pct;
      // 健康度 / 容量（能读到才显示，不给假数字）
      const healthTxt = (b.health >= 0) ? `健康度 ${b.health}%` : '';
      const capTxt = (b.full_mwh > 0) ? `满充 ${b.full_mwh} mWh` : '';
      const meta2 = [b.conn || '其它', healthTxt, capTxt].filter(Boolean).join(' · ');
      const row = document.createElement('div');
      row.className = 'batt-row';
      row.innerHTML = `
        <div class="batt-icon">${battIcon(b.conn)}</div>
        <div class="batt-main">
          <div class="batt-name">${b.name || '电池设备'}</div>
          <div class="batt-meta">${meta2}</div>
        </div>
        <div class="batt-right">
          <div class="batt-bar"><i class="${barCls}" style="width:${barWidth}%"></i></div>
          <div class="batt-pct">${pct == null ? '—' : pct + '%'}</div>
          <div class="batt-state ${st.cls}">${st.txt}</div>
        </div>
      `;
      box.appendChild(row);
    });
  } catch (e) {
    card.style.display = '';
    box.innerHTML = '<div class="empty">读取电池设备失败：' + errText(e) + '</div>';
  }
}

// ===== 按平台调整能力（各平台支持范围不同，只改行为不改说明文案） =====
function applyPlatformCopy(p) {
  activePlatform = p;
  const matchMac = document.getElementById('match-mac');
  const hdrButton = document.getElementById('hdr-sync');
  const grayOption = document.querySelector('#color-space option[value="gray"]');
  if (p === 'windows') {
    if (hdrButton) hdrButton.disabled = false;
    // Windows 没有内建 P3 屏，「对齐 Mac 内建屏」改为语义成立的「对齐主屏色彩」
    matchMac.textContent = '对齐主屏色彩';
    matchMac.title = '把主屏当前关联的 ICC 配置应用到其余屏幕';
    if (grayOption) {
      grayOption.disabled = true;
      grayOption.title = 'Windows RGB 显示器不能安全使用灰度 ICC 配置';
    }
  } else if (p === 'linux') {
    matchMac.disabled = true;
    matchMac.title = 'Linux 版暂未实现';
    const apply = document.getElementById('apply-color');
    if (apply) { apply.disabled = true; apply.title = 'Linux 版暂未实现'; }
    if (hdrButton) { hdrButton.disabled = true; setBtnLabel('hdr-sync', 'HDR 不支持'); }
  } else {
    if (grayOption) {
      grayOption.disabled = false;
      grayOption.title = '';
    }
    matchMac.title = '外接屏对齐 Mac 内建屏（Display P3）';
    const applyColor = document.getElementById('apply-color');
    if (applyColor) { applyColor.disabled = false; applyColor.title = '通过 macOS ColorSync 设置系统显示器配置'; }
    matchMac.disabled = false;
    if (hdrButton) {
      hdrButton.disabled = true;
      setBtnLabel('hdr-sync', 'HDR 不支持');
      hdrButton.title = 'macOS HDR 由系统和显示器驱动管理，Screenguard 当前不提供切换';
    }
  }
}

// ===== 按钮事件 =====
function errText(e) {
  if (typeof e === 'string') return e;
  if (e && e.message) return e.message;
  return '未知错误';
}
async function run(cmd, args, okMsg) {
  try { await invoke(cmd, args); setStatus(okMsg); }
  catch (e) { setStatus('失败：' + errText(e), false); }
}

document.getElementById('apply-color').onclick = () => {
  const space = document.getElementById('color-space').value;
  run('apply_color_space', { space }, `已应用 ${space} 到所有屏幕`);
};
document.getElementById('match-mac').onclick = () => {
  const label = document.getElementById('match-mac').textContent;
  run('match_mac', {}, `已${label}`);
};

// 亮度同步：以「主屏亮度」为基准，把其它能读到亮度的屏对齐过去。
// 读不到独立亮度的屏（DDC 不通，如小米）由「总亮度」统一控制 —— 如实说明。
document.getElementById('bri-sync').onclick = async () => {
  try {
    setStatus('正在同步各屏亮度…');
    const ds = await invoke('get_displays') || [];
    const withBri = ds.filter(d => d.brightness != null);
    if (!withBri.length) {
      setStatus('两块屏都读不到独立亮度（DDC 不通）→ 请用「总亮度」统一调节（对不支持的屏也生效）', false);
      return;
    }
    const ref = withBri.find(d => d.main) || withBri[0];
    const target = Number(ref.brightness);
    const changed = [];
    for (const d of withBri) {
      if (Number(d.brightness) === target) continue;
      try { await invoke('set_brightness', { displayId: d.id, value: target }); changed.push(d.name || d.id); } catch (e) {}
    }
    const unread = ds.length - withBri.length;
    let msg = `以「${ref.name || '主屏'}」的 ${target}% 为基准：` + (changed.length ? `已调整 ${changed.join('、')}` : '各可读屏已是同一值');
    if (unread > 0) msg += `；另有 ${unread} 台屏读不到独立亮度（DDC 不通），由「总亮度」统一控制`;
    setStatus(msg);
  } catch (e) { setStatus('亮度同步失败：' + errText(e), false); }
};

// HDR：按钮直接显示**当前状态**（开 / 关 / 混合），点一下才把所有屏设成一致。
// 说明：本机实测 Windows 的 HDR 设置 API 返回成功但状态可能不变（被厂商/驱动接管），
// 所以结果里会带上逐台原始明细，不给你一个漂亮但不实的结论。
let activePlatform = '';
let hdrStat = { sup: 0, on: 0, total: 0 };
async function loadHdrStatus() {
  const btn = document.getElementById('hdr-sync');
  if (!btn) return;
  if (activePlatform === 'macos') {
    btn.disabled = true;
    setBtnLabel('hdr-sync', 'HDR 不支持');
    return;
  }
  try {
    const raw = String(await invoke('hdr_states') || '');
    if (activePlatform === 'macos') {
      btn.disabled = true;
      setBtnLabel('hdr-sync', 'HDR 不支持');
      return;
    }
    let sup = 0, on = 0, total = 0;
    for (const line of raw.split('\n')) {
      const p = line.split('|');
      if (p[0] === 'H' && p.length >= 4) {
        total++;
        if (p[2] === '1') sup++;
        if (p[3] === '1') on++;
      }
    }
    hdrStat = { sup, on, total };
    let txt;
    if (!sup) txt = 'HDR 不支持';
    else if (on === sup) txt = 'HDR 开';
    else if (on === 0) txt = 'HDR 关';
    else txt = `HDR 混合 ${on}/${sup}`;
    setBtnLabel('hdr-sync', txt);
    btn.title = '点一下把所有屏的 HDR 设成一致';
  } catch (e) {
    if (activePlatform === 'macos') {
      btn.disabled = true;
      setBtnLabel('hdr-sync', 'HDR 不支持');
      return;
    }
    setBtnLabel('hdr-sync', 'HDR 读取失败');
  }
}
document.getElementById('hdr-sync').onclick = async () => {
  try {
    setStatus('正在切换 HDR…');
    const want = !(hdrStat.sup > 0 && hdrStat.on === hdrStat.sup);
    const r = await invoke('hdr_set', { on: want });
    setStatus(String(r));
    await loadHdrStatus();
  } catch (e) { setStatus('HDR 切换失败：' + errText(e), false); }
};
loadHdrStatus();

// 总亮度（gamma，对所有屏生效）
const bri = { latest: null, busy: false };
function sendGamma(v) {
  bri.latest = v;
  if (bri.busy) return;
  bri.busy = true;
  (async () => {
    while (bri.latest !== null) {
      const v2 = bri.latest; bri.latest = null;
      try { await invoke('gamma_set', { pct: Number(v2) }); } catch (e) {}
    }
    bri.busy = false;
  })();
}
document.getElementById('all-bri').addEventListener('input', (ev) => {
  document.getElementById('all-bri-val').textContent = ev.target.value + '%';
  setStatus('总亮度 → ' + ev.target.value + '%');
  sendGamma(ev.target.value);
});
document.getElementById('bri-reset').onclick = async () => {
  document.getElementById('all-bri').value = 100;
  document.getElementById('all-bri-val').textContent = '100%';
  try { await invoke('gamma_set', { pct: 100 }); setStatus('总亮度已复位到 100%（原始曲线）'); }
  catch (e) { setStatus('复位失败：' + errText(e), false); }
};
async function loadGamma() {
  try {
    const g = String(await invoke('gamma_get') || '');
    let pct = 100;
    for (const line of g.split('\n')) {
      const p = line.split('|');
      if (p[0] === 'G' && p.length >= 3) { const v = Number(p[2]); if (v >= 40 && v !== 100) pct = v; }
    }
    document.getElementById('all-bri').value = pct;
    document.getElementById('all-bri-val').textContent = pct + '%';
  } catch (e) {}
}
loadGamma();

// 全局快捷键显示
(async () => {
  const el = document.getElementById('hk-note');
  if (!el) return;
  try {
    const [hk, platform] = await Promise.all([invoke('hotkey_start'), getPlatform()]);
    const restoreKey = platform === 'macos' ? 'Control+Option+R' : 'Ctrl+Alt+R';
    el.innerHTML = `<kbd>${hk}</kbd> 待机 / 唤醒 <span class="sep">·</span> <kbd>${restoreKey}</kbd> 还原窗口`;
  } catch (e) { el.textContent = errText(e); }
})();

// 铺满任意前台窗口
// 铺满方式：2 = 自动对齐后铺满；1 = 主屏尺寸；0 = 所有屏幕包围盒
function spanMode() {
  const el = document.getElementById('span-mode');
  const value = Number(el && el.value);
  return [0, 1, 2].includes(value) ? value : 1;
}

document.getElementById('span-fg').onclick = async () => {
  try {
    if (spanMode() === 2 && await getPlatform().catch(() => '') === 'macos'
      && !confirm('自动对齐会临时切换两块屏的分辨率，画面可能短暂变黑且清晰度会降低。\n\n原始分辨率已备份；点「还原」或退出软件可恢复。继续吗？')) return;
    setStatus('正在把当前窗口铺满两块屏…');
    const title = await invoke('span_foreground', { mode: spanMode() });
    setStatus(`已铺满：「${title}」（点「还原」回到原位置）`);
  } catch (e) { setStatus('铺满失败：' + errText(e), false); }
};
document.getElementById('restore-fg').onclick = async () => {
  try { await invoke('restore_foreground'); setStatus('已还原上一个被铺满的窗口'); }
  catch (e) { setStatus('还原失败：' + errText(e), false); }
};
// ⚠️ 重要：Windows 改「每屏缩放」只能写注册表，**必须注销或重新登录才生效**，
// 没有运行时 API 能立刻变。所以点完当场看不出任何变化是正常的 —— 这里明说，免得白点。
document.getElementById('match-ppi').onclick = async () => {
  try {
    setStatus('正在写入缩放档…（写入后需注销/重新登录才生效）');
    const msg = await invoke('match_dpi_apply');
    setStatus(msg + '（需注销或重新登录才生效，原值已备份）');
  } catch (e) { setStatus('对齐失败：' + errText(e), false); }
};
document.getElementById('dpi-restore').onclick = async () => {
  try {
    const msg = await invoke('match_dpi_restore');
    setStatus(msg + '（同样需要注销或重新登录才生效）');
  } catch (e) { setStatus('还原失败：' + errText(e), false); }
};
// 副屏横竖屏：按钮直接显示**当前方向**，点一下切换
// （原「切换 + 恢复副屏竖屏」两个按钮已合并成一个，避免用户记不清该点哪个）
async function loadRotateStatus() {
  const btn = document.getElementById('rotate-secondary');
  if (!btn) return;
  try {
    const ds = await invoke('get_displays') || [];
    const sec = ds.find(d => !d.main) || ds[0];
    if (!sec) { setBtnLabel('rotate-secondary', '未检测到副屏'); return; }
    // 后端返回的是 resolution 字符串（如 "2400x3840"），不是 w/h 字段
    let W = Number(sec.w != null ? sec.w : sec.width);
    let H = Number(sec.h != null ? sec.h : sec.height);
    if (!W || !H) {
      const m = String(sec.resolution || '').match(/^(\d+)\s*x\s*(\d+)/i);
      if (m) { W = Number(m[1]); H = Number(m[2]); }
    }
    const portrait = H > W;
    setBtnLabel('rotate-secondary', `副屏${portrait ? '竖屏' : '横屏'}${W ? ` ${W}×${H}` : ''}`);
    btn.title = '点击切换副屏方向';
    btn.dataset.portrait = portrait ? '1' : '0';
  } catch (e) { setBtnLabel('rotate-secondary', '方向读取失败'); }
}
document.getElementById('rotate-secondary').onclick = async () => {
  const btn = document.getElementById('rotate-secondary');
  const wasPortrait = btn.dataset.portrait === '1';
  btn.disabled = true;
  try {
    setStatus('正在切换副屏方向…（屏幕会短暂黑一下，稍等）');
    // 注意方向别接反：现在**竖屏**时要的是横屏 → 调 rotate_secondary（它把副屏转成横屏）；
    // 现在**横屏**时才调 restore_secondary（它把副屏恢复成竖屏）。
    // 之前写反了，于是「本来竖屏、点一下又叫它竖屏」，用户看到的就是「点了没反应」。
    await invoke(wasPortrait ? 'rotate_secondary' : 'restore_secondary');
    setStatus(wasPortrait ? '已把副屏切为横屏' : '已把副屏切为竖屏');
    setTimeout(loadRotateStatus, 2600);
  } catch (e) { setStatus('切换失败：' + errText(e), false); }
  finally { setTimeout(() => { btn.disabled = false; }, 1800); }
};
loadRotateStatus();
// ===== v0.3.3：窗口选择器 / 恢复默认 / 输出设备选择 =====
async function loadSpanTargets() {
  const sel = document.getElementById('span-target');
  if (!sel) return;
  try {
    setStatus('正在读取窗口列表…');
    const raw = String(await invoke('list_windows') || '');
    const rows = [];
    const permissions = new Set();
    for (const line of raw.split('\n')) {
      const p = line.split('|');
      if (p[0] === 'W' && p.length >= 4) rows.push({ hwnd: p[1], size: p[2], title: p.slice(3).join('|') });
      if (p[0] === 'I' && p[1] === 'permission') permissions.add(p[2]);
    }
    sel.innerHTML = rows.length
      ? rows.map(r => `<option value="${r.hwnd}">${r.title}　(${r.size})</option>`).join('')
      : '<option value="">（没有可铺满的窗口）</option>';
    const hints = [];
    if (permissions.has('screen-capture')) hints.push('允许「屏幕与系统音频录制」以读取窗口标题');
    if (permissions.has('accessibility')) hints.push('允许「辅助功能」以移动和调整窗口');
    setStatus(`找到 ${rows.length} 个可铺满的窗口${hints.length ? '；macOS 权限待配置：' + hints.join('；') : '，选中后点「铺满选中窗口」'}`, !hints.includes('允许「辅助功能」以移动和调整窗口'));
  } catch (e) {
    sel.innerHTML = '<option value="">（读取失败）</option>';
    setStatus('读取窗口列表失败：' + errText(e), false);
  }
}
document.getElementById('span-refresh').onclick = () => loadSpanTargets();
document.getElementById('span-pick').onclick = async () => {
  const sel = document.getElementById('span-target');
  const hwnd = sel && sel.value ? Number(sel.value) : 0;
  if (!hwnd) { setStatus('请先点「刷新窗口列表」并选一个窗口', false); return; }
  try {
    if (spanMode() === 2 && await getPlatform().catch(() => '') === 'macos'
      && !confirm('自动对齐会临时切换两块屏的分辨率，画面可能短暂变黑且清晰度会降低。\n\n原始分辨率已备份；点「还原」或退出软件可恢复。继续吗？')) return;
    setStatus('正在铺满…');
    const title = await invoke('span_window', { hwnd, mode: spanMode() });
    const p = await getPlatform().catch(() => '');
    setStatus(p === 'macos'
      ? `已铺满：「${title}」。点「↩ 还原」回到原位置`
      : `已铺满：「${title}」。还原：点「↩ 还原」或按 Ctrl+Alt+R`);
  } catch (e) { setStatus('铺满失败：' + errText(e), false); }
};
document.getElementById('restore-defaults').onclick = async () => {
  try {
    setStatus('正在恢复到默认状态…');
    const msg = await invoke('restore_defaults');
    setStatus(String(msg));
    await loadAudio(); await loadGamma();
  } catch (e) { setStatus('恢复默认失败：' + errText(e), false); }
};
(async () => {
  try {
    const st = JSON.parse(await invoke('app_state_json') || '{}');
    const cb = document.getElementById('revert-exit');
    if (cb) cb.checked = st.revert_on_exit !== false;
  } catch (e) {}
})();
(async () => {
  const cb = document.getElementById('revert-exit');
  if (!cb) return;
  cb.onchange = async () => {
    try {
      await invoke('set_revert_on_exit', { on: cb.checked });
      setStatus(cb.checked ? '已开启：退出软件时自动还原' : '已关闭：退出软件时保留当前设置');
    } catch (e) { setStatus('设置失败：' + errText(e), false); }
  };
})();
(async () => {
  const sel = document.getElementById('audio-dev-sel');
  if (!sel) return;
  sel.onchange = async () => {
    try {
      await invoke('set_audio_device', { id: sel.value });
      await loadAudio();
      setStatus('已切换控制「' + (sel.options[sel.selectedIndex] || {}).text + '」');
    } catch (e) { setStatus('切换输出设备失败：' + errText(e), false); }
  };
})();

// ===== 小米屏音量（走显示器自己的 MiTV 接口：DDC 不通、电脑侧音频端点又是固定音量）=====
async function loadMitvVolume() {
  const row = document.getElementById('mitv-row');
  if (!row) return;
  try {
    const ok = await invoke('mitv_available');
    if (!ok) { row.style.display = 'none'; return; }
    const v = await invoke('mitv_volume_get');
    row.style.display = '';
    document.getElementById('mitv-vol').value = v;
    document.getElementById('mitv-vol-val').textContent = v + '%';
  } catch (e) {
    row.style.display = 'none';
  }
}
(function () {
  const sl = document.getElementById('mitv-vol');
  if (!sl) return;
  let deb = null;
  let poll = null;
  sl.addEventListener('input', (ev) => {
    const v = Number(ev.target.value);
    document.getElementById('mitv-vol-val').textContent = v + '%';
    // 只更新「目标」，不等待：后端拿到最新目标后自己逐步追赶。
    // 这样「拖回 20%」会立刻反向，不会先把旧目标走完再降（用户实测的痛点）。
    invoke('mitv_volume_set', { target: v }).catch(() => {});
    if (deb) clearTimeout(deb);
    // 每隔 600ms 就把**显示值**校准成显示器真实音量，直到追上目标为止。
    // 之前只在 1 秒后读一次，而步进是 1% 一步（220ms/步），大跨度要好几秒才走完，
    // 所以界面看起来「很久没反应」（用户实测）—— 现在全程跟着走，实时能看见数字在涨。
    if (!poll) {
      poll = setInterval(async () => {
        try {
          const now = await invoke('mitv_volume_get');
          sl.value = now;
          document.getElementById('mitv-vol-val').textContent = now + '%';
          if (Number(now) === Number(sl.dataset.want || now)) {
            clearInterval(poll); poll = null;
          }
        } catch (e) { clearInterval(poll); poll = null; }
      }, 600);
    }
    sl.dataset.want = String(v);
    if (deb) clearTimeout(deb);
    deb = setTimeout(() => { if (poll) { clearInterval(poll); poll = null; } }, 8000);
  });
  loadMitvVolume();
})();

// ===== 一键检测 =====
(function () {
  const btn = document.getElementById('diag-run');
  const open = document.getElementById('diag-open');
  const pre = document.getElementById('diag-out');
  const when = document.getElementById('diag-when');
  // 日志路径放进按钮提示，不再占一行小字
  (async () => {
    if (!open) return;
    try { open.title = '日志目录：' + await invoke('diag_log_path'); }
    catch (e) { open.title = '打开日志文件夹'; }
  })();
  if (open) open.onclick = async () => {
    try { await invoke('open_log_dir'); setStatus('已打开日志文件夹'); }
    catch (e) { setStatus('打开失败：' + errText(e), false); }
  };
  const fix = document.getElementById('diag-fix');
  if (fix) fix.onclick = async () => {
    fix.disabled = true;
    try {
      const r = await invoke('diag_fix');
      pre.textContent = String(r);
      if (when) when.textContent = '上次修复+复查：' + new Date().toLocaleTimeString();
      setStatus('修复完成，已写入日志');
    } catch (e) {
      pre.textContent = '修复失败：' + errText(e);
      setStatus('修复失败', false);
    } finally { fix.disabled = false; }
  };
  if (btn) btn.onclick = async () => {
    btn.disabled = true;
    pre.textContent = '正在检测…';
    try {
      const r = await invoke('diag_scan');
      pre.textContent = String(r);
      if (when) when.textContent = '上次检测：' + new Date().toLocaleTimeString();
      setStatus('检测完成，已写入日志');
    } catch (e) {
      pre.textContent = '检测失败：' + errText(e);
      setStatus('检测失败', false);
    } finally { btn.disabled = false; }
  };
})();
// ===== 双屏对齐：应用 / 还原 =====
// 应用=把两屏切到"竖向高度一致"的可行组合；还原=回到原始 4K。
// 关闭双屏铺满 / 退出软件 / 一键恢复默认 都会自动触发还原（后端已接好）。
(function () {
  const ap = document.getElementById('align-apply');
  const rs = document.getElementById('align-restore');
  const out = document.getElementById('align-out');
  if (ap) ap.onclick = async () => {
    const platform = await getPlatform().catch(() => '');
    const ok = confirm(platform === 'macos'
      ? '应用推荐的双屏分辨率组合？\n\n· 屏幕会短暂变黑，分辨率可能明显降低\n· 系统会先保存原始模式；失败时自动尝试回滚\n· 可用「还原」或退出软件恢复原分辨率\n\n继续吗？'
      : '把两块屏切到「竖向高度一致」的分辨率组合？\n\n· 切换时屏幕会黑一下（正常）\n· 已自动记录原始分辨率，可一键还原\n· 若切换失败会自动回滚\n\n继续吗？');
    if (!ok) return;
    ap.disabled = true;
    if (out) out.textContent = '正在试算并切换…（屏幕可能短暂黑屏）';
    try {
      const r = await invoke('align_apply');
      if (out) out.textContent = String(r);
      setStatus('双屏已对齐；关闭双屏铺满或退出软件可恢复原分辨率');
    } catch (e) {
      if (out) out.textContent = '对齐失败：' + errText(e);
      setStatus('对齐失败', false);
    } finally { ap.disabled = false; }
  };
  if (rs) rs.onclick = async () => {
    rs.disabled = true;
    try {
      const r = await invoke('align_restore');
      if (out) out.textContent = String(r);
      setStatus(String(r).slice(0, 90));
    } catch (e) { setStatus('还原失败：' + errText(e), false); }
    finally { rs.disabled = false; }
  };
})();

// ===== 双屏对齐体检 =====
(function () {
  const btn = document.getElementById('align-run');
  const pre = document.getElementById('align-out');
  const save = document.getElementById('align-save');
  const mf = document.getElementById('align-manuf');
  const inch = document.getElementById('align-inches');
  const sizes = document.getElementById('align-sizes');

  async function paintSizes() {
    try {
      const raw = await invoke('app_state_json');
      const st = JSON.parse(String(raw));
      const arr = (st && st.screen_in) || [];
      sizes.textContent = arr.length
        ? '已记录尺寸：' + arr.map(s => s.replace('|', ' → ') + ' 英寸').join('　')
        : '';
    } catch (e) { sizes.textContent = ''; }
  }
  if (btn) btn.onclick = async () => {
    btn.disabled = true;
    pre.textContent = '正在体检…';
    try {
      pre.textContent = String(await invoke('align_report'));
      setStatus('体检完成');
    } catch (e) { pre.textContent = '体检失败：' + errText(e); }
    finally { btn.disabled = false; }
  };
  const cp = document.getElementById('align-copy');
  if (cp) cp.onclick = async () => {
    try {
      await navigator.clipboard.writeText(pre.textContent || '');
      setStatus('结论已复制到剪贴板');
    } catch (e) { setStatus('复制失败（可手动选中复制）', false); }
  };
  if (save) save.onclick = async () => {
    const m = (mf.value || '').trim().toUpperCase();
    const v = Number(inch.value);
    if (!m || !(v > 5 && v < 60)) { setStatus('请填：厂商代码（如 MTI1B1A）+ 对角线英寸（如 16）', false); return; }
    try {
      await invoke('set_screen_inches', { manuf: m, inches: v });
      setStatus('已记录 ' + m + ' = ' + v + ' 英寸');
      mf.value = ''; inch.value = '';
      await paintSizes();
    } catch (e) { setStatus('保存失败：' + errText(e), false); }
  };
  getPlatform().then(platform => {
    if (platform === 'macos') {
      const modeActions = document.getElementById('align-mode-actions');
      const manualSize = document.getElementById('align-manual-size');
      if (modeActions) modeActions.style.display = '';
      if (manualSize) manualSize.style.display = 'none';
      if (sizes) sizes.textContent = '';
      return;
    }
    paintSizes();
  }).catch(() => paintSizes());
})();

document.getElementById('refresh-btn').onclick = () => refreshDisplays();
document.getElementById('battery-refresh-btn').onclick = () => loadBatteries();

// 系统音量（Windows 才显示；在 startup 里按平台加载）
document.getElementById('sys-vol').addEventListener('input', (ev) => {
  document.getElementById('sys-vol-val').textContent = ev.target.value + '%';
  setStatus('系统音量 → ' + ev.target.value);
  sendSysVol(ev.target.value);
});
document.getElementById('mute-btn').onclick = async () => {
  const btn = document.getElementById('mute-btn');
  try {
    const want = !sysVol.muted;
    await invoke('set_system_mute', { on: want });
    sysVol.muted = want;
    btn.textContent = want ? '取消静音' : '静音';
    setStatus(want ? '已静音' : '已取消静音');
  } catch (e) {
    setStatus('静音切换失败：' + errText(e), false);
  }
};

// ===== 启动 =====
(async function startup() {
  try {
    const p = await getPlatform();
    const names = { macos: 'macOS', windows: 'Windows', linux: 'Linux' };
    setBadge(names[p] || p);
    applyPlatformCopy(p);
    if (p === 'macos') {
      document.getElementById('native-display-card').style.display = '';
      document.getElementById('diag-card').style.display = 'none';
      const summary = document.getElementById('restore-summary');
      if (summary) summary.textContent = '恢复本机已记录且系统允许恢复的状态';
    }
    if (p !== 'windows') document.getElementById('diag-card').style.display = 'none';
    if (p === 'windows' || p === 'macos') {
      // Windows 枚举 CoreAudio 端点；macOS 仅控制系统默认输出。
      document.getElementById('audio-card').style.display = '';
      await loadAudio();
    }
    if (p === 'windows') {
      // 电池设备卡：仅 Windows（Battery 设备类枚举）
      await loadBatteries();
    }
  } catch (e) {
    if (e.message === 'TAURI_CORE_UNAVAILABLE') setBadge('预览模式');
    else setBadge('出错了');
  }
  try {
    const v = await invoke('get_version');
    const verEl = document.getElementById('version');
    if (verEl) verEl.textContent = 'v' + v;
    checkForUpdates(false, v, true);
  } catch (e) { /* 预览模式：静默 */ }
  await refreshDisplays();
})();
