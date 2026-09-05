// 屏幕守护 — 跨平台前端逻辑 (Tauri 2)
// 通过 window.__TAURI__.core.invoke 调用 Rust 后端命令

const statusEl = document.getElementById('status');
const platformBadge = document.getElementById('platform-badge');

function setStatus(msg, ok = true) {
  statusEl.textContent = msg;
  statusEl.style.color = ok ? 'var(--text-dim)' : 'var(--err)';
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
    box.innerHTML = '<div class="empty">未检测到显示器</div>';
    return;
  }
  box.innerHTML = ''; // 清空 + 重建（保留简洁结构）

  displays.forEach(d => {
    const row = document.createElement('div');
    row.className = 'display-row';

    const ddcBadge = d.ddc ? '<span class="badge ddc">DDC</span>' : '';
    const connectedBadge = d.connected
      ? '<span class="badge on">已连接</span>'
      : '<span class="badge off">未连接</span>';

    row.innerHTML = `
      <div class="display-head">
        <div>
          <div class="display-name">${d.name || d.id}</div>
          <div class="display-meta">${d.resolution || ''} · ${d.hz ? d.hz + 'Hz' : ''}${d.main ? ' · 主屏' : ''}${d.color_profile ? ' · 色彩:' + d.color_profile : ''}</div>
        </div>
        <div style="display:flex;gap:6px">${ddcBadge}${connectedBadge}</div>
      </div>
      ${d.ddc ? `
      <div class="slider-row">
        <label>亮度</label>
        <input type="range" min="0" max="100" value="${d.brightness ?? 50}" data-display="${d.id}" data-kind="brightness">
        <span class="val">${d.brightness ?? 50}</span>
      </div>
      <div class="slider-row">
        <label>音量</label>
        <input type="range" min="0" max="100" value="${d.volume ?? 50}" data-display="${d.id}" data-kind="volume">
        <span class="val">${d.volume ?? 50}</span>
      </div>
      ` : '<div class="hint">此显示器不支持 DDC/CI，请用硬件按钮，或使用软件校准。</div>'}
    `;
    box.appendChild(row);
  });

  // 绑定滑块
  box.querySelectorAll('input[type=range]').forEach(sl => {
    sl.addEventListener('input', async (ev) => {
      const valEl = ev.target.parentElement.querySelector('.val');
      if (valEl) valEl.textContent = ev.target.value;
      const kind = ev.target.dataset.kind;
      const dispId = ev.target.dataset.display;
      try {
        if (kind === 'brightness') await invoke('set_brightness', { displayId: dispId, value: Number(ev.target.value) });
        else await invoke('set_volume', { displayId: dispId, value: Number(ev.target.value) });
        setStatus(`${kind === 'brightness' ? '亮度' : '音量'} → ${ev.target.value}`);
      } catch (e) {
        setStatus('控制失败：' + e.message, false);
      }
    });
  });
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
document.getElementById('match-mac').onclick = () => run('match_mac', {}, '已对齐 Mac 内建屏');
document.getElementById('match-ppi').onclick = () => run('match_ppi', {}, '已匹配 PPI（窗口跨屏不再变小）');
document.getElementById('rotate-secondary').onclick = () => run('rotate_secondary', {}, '已切换副屏横竖屏');
document.getElementById('restore-secondary').onclick = () => run('restore_secondary', {}, '已恢复副屏为竖屏');
document.getElementById('span-video').onclick = () => run('span_video', {}, '正在用 VLC 铺满双屏…');
document.getElementById('restore-video').onclick = () => run('restore_video', {}, '已恢复播放器窗口');
document.getElementById('refresh-btn').onclick = () => refreshDisplays();

// ===== 启动 =====
(async function startup() {
  try {
    const p = await invoke('get_platform');
    const names = { macos: 'macOS', windows: 'Windows', linux: 'Linux' };
    setBadge(names[p] || p);
  } catch (e) {
    if (e.message === 'TAURI_CORE_UNAVAILABLE') setBadge('预览模式');
    else setBadge('出错了');
  }
  await refreshDisplays();
})();
