// 屏幕守护 — 跨平台前端逻辑 (Tauri 2)
// 通过 window.__TAURI__.core.invoke 调用 Rust 后端命令

const statusEl = document.getElementById('status');
const platformBadge = document.getElementById('platform-badge');

function setStatus(msg, ok = true) {
  statusEl.textContent = msg;
  statusEl.style.color = ok ? '' : '#ef6a6a';
}

function setBadge(txt) { platformBadge.textContent = txt; }

function invoke(cmd, args = {}) {
  // Tauri 2：核心 API 暴露在 window.__TAURI__.core
  if (window.__TAURI__ && window.__TAURI__.core) {
    return window.__TAURI__.core.invoke(cmd, args);
  }
  // 调试/无 Tauri 环境：返回假数据，避免页面空白
  return Promise.reject(new Error('TAURI_CORE_UNAVAILABLE'));
}

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

async function refreshDisplays() {
  try {
    setStatus('读取显示器…');
    const displays = await invoke('get_displays');
    renderDisplays(displays);
    setStatus('就绪');
  } catch (e) {
    renderDisplays([]);
    setStatus(e.message === 'TAURI_CORE_UNAVAILABLE' ? '非 Tauri 环境（预览模式）' : '读取显示器失败：' + e.message, false);
  }
}

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
    let hint = '';
    if (!hasBri && !d.ddc) {
      hint = '<div class="hint">此屏不支持 DDC/CI 也无 WMI 亮度：请用硬件按键调节。</div>';
    } else if (hasBri && !d.ddc) {
      hint = '<div class="hint">此屏无 DDC/CI（典型为笔记本内屏）：亮度走系统 WMI；喇叭/耳机音量请用「系统音量」。</div>';
    }
    const degradeHint = degraded ? `
      <div class="hint" style="color:#e8a75c">⚠️ 仅一块屏且运行在 1024x768@60 —— 显示驱动栈很可能异常（此前实测 NVIDIA 驱动崩溃循环后即此形态，亮度/音量会全部失效）。按 Win+Ctrl+Shift+B 重启图形驱动，画面恢复后点「刷新」。</div>` : '';

    row.innerHTML = `
      <div class="display-head">
        <div>
          <div class="display-name">${d.name || d.id}</div>
          <div class="display-meta">${d.resolution || ''} · ${d.hz ? d.hz + 'Hz' : ''}${d.main ? ' · 主屏' : ''}${d.color_profile ? ' · 色彩:' + d.color_profile : ''}</div>
        </div>
        <div style="display:flex;gap:6px">${briBadge}${connectedBadge}</div>
      </div>
      ${briRow}${volRow}${hint}${degradeHint}
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

async function loadAudio() {
  const card = document.getElementById('audio-card');
  const devEl = document.getElementById('audio-dev');
  const hint = document.getElementById('audio-hint');
  const sl = document.getElementById('sys-vol');
  const val = document.getElementById('sys-vol-val');
  const muteBtn = document.getElementById('mute-btn');
  try {
    const a = await invoke('get_system_audio');
    card.style.display = '';
    devEl.textContent = a.name || '默认输出设备';
    sl.disabled = false;
    sl.value = a.volume;
    val.textContent = a.volume + '%';
    sysVol.muted = a.mute;
    muteBtn.textContent = a.mute ? '🔕 取消静音' : '🔇 静音';
    if (!a.adjustable) {
      sl.disabled = true;
      hint.textContent = '此输出端点为固定音量（Windows 滑块也无效，常见于部分 HDMI/DP 显示器音频）。若在用显示器喇叭，请用显示器条目里的 DDC 音量滑块；或改用其它输出设备。';
      muteBtn.disabled = true;
    } else {
      muteBtn.disabled = false;
    }
  } catch (e) {
    card.style.display = '';
    devEl.textContent = '不可用';
    hint.textContent = '读取系统音量失败：' + errText(e);
  }
}

// ===== 按平台调整文案（各平台能力/依赖不同，避免误导） =====
function applyPlatformCopy(p) {
  const matchMac = document.getElementById('match-mac');
  const note = document.getElementById('color-note');
  if (p === 'windows') {
    // Windows 没有内建 P3 屏，「对齐 Mac 内建屏」改为语义成立的「对齐主屏色彩」
    matchMac.textContent = '对齐主屏色彩';
    matchMac.title = '把主屏当前关联的 ICC 配置应用到其余屏幕';
    if (note) note.textContent = 'P3 / Adobe RGB 需自备 .icc 放到 %LOCALAPPDATA%\\screenguard_icc｜变更显示器色彩配置需管理员权限';
  } else if (p === 'linux') {
    matchMac.disabled = true;
    matchMac.title = 'Linux 版暂未实现';
    const apply = document.getElementById('apply-color');
    if (apply) { apply.disabled = true; apply.title = 'Linux 版暂未实现'; }
    if (note) note.textContent = 'Linux 版色彩同步暂未实现（可先用系统级 colord 配置）';
  } else {
    matchMac.title = '外接屏对齐 Mac 内建屏（Display P3）';
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
document.getElementById('match-ppi').onclick = () => run('match_ppi', {}, '已匹配 PPI（窗口跨屏不再变小）');
document.getElementById('rotate-secondary').onclick = () => run('rotate_secondary', {}, '已切换副屏横竖屏');
document.getElementById('restore-secondary').onclick = () => run('restore_secondary', {}, '已恢复副屏为竖屏');
document.getElementById('span-video').onclick = () => run('span_video', {}, '正在铺满双屏…');
document.getElementById('restore-video').onclick = () => run('restore_video', {}, '已恢复播放器窗口');
document.getElementById('refresh-btn').onclick = () => refreshDisplays();

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
    btn.textContent = want ? '🔕 取消静音' : '🔇 静音';
    setStatus(want ? '已静音' : '已取消静音');
  } catch (e) {
    setStatus('静音切换失败：' + errText(e), false);
  }
};

// ===== 启动 =====
(async function startup() {
  try {
    const p = await invoke('get_platform');
    const names = { macos: 'macOS', windows: 'Windows', linux: 'Linux' };
    setBadge(names[p] || p);
    applyPlatformCopy(p);
    if (p === 'windows') {
      // 系统音量卡：仅 Windows（CoreAudio 默认输出端点）
      document.getElementById('audio-card').style.display = '';
      await loadAudio();
    }
  } catch (e) {
    if (e.message === 'TAURI_CORE_UNAVAILABLE') setBadge('预览模式');
    else setBadge('出错了');
  }
  try {
    const v = await invoke('get_version');
    const verEl = document.getElementById('version');
    if (verEl) verEl.textContent = 'v' + v;
  } catch (e) { /* 预览模式：静默 */ }
  await refreshDisplays();
})();
