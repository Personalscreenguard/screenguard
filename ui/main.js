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

// ===== 屏幕电源（纯本地，不依赖网络）=====
// 全部待机走 Windows 系统级指令：一次黑掉全部屏，含不走 DDC 的屏。
const pwNote = document.getElementById('pw-note');
function pwStat(t, ok = true) { pwNote.textContent = t; pwNote.style.color = ok ? '' : '#ef6a6a'; }

document.getElementById('pw-off').addEventListener('click', async () => {
  pwStat('正在让全部屏幕待机…');
  try { await invoke('screen_off'); pwStat('已待机（动一下鼠标或按键盘即可唤醒）'); }
  catch (e) { pwStat('待机失败：' + (e && e.message ? e.message : e), false); }
});

document.getElementById('pw-wake').addEventListener('click', async () => {
  try { await invoke('screen_wake'); pwStat('已唤醒屏幕'); }
  catch (e) { pwStat('唤醒失败：' + (e && e.message ? e.message : e), false); }
});

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
  const sel = document.getElementById('audio-dev-sel');
  try {
    card.style.display = '';
    // v0.3.3：音量作用于「用户选定的输出端点」。默认输出若正好是固定音量的
    // HDMI/DP 显示器音频（Windows 自己的滑块也调不动），就自动落在一个可调的
    // 端点上，并把设备列出来让用户自己换 —— 不再出现「滑块动不了」的死路。
    const eps = await invoke('audio_endpoints');
    let st = {};
    try { st = JSON.parse(await invoke('app_state_json') || '{}'); } catch (e) {}
    const wanted = eps.find(e => e.id === st.audio_dev);
    const cur = wanted || eps.find(e => e.is_default) || eps.find(e => e.adjustable) || eps[0];
    if (sel) {
      sel.innerHTML = eps.map(e =>
        `<option value="${e.id}"${(cur && e.id === cur.id) ? ' selected' : ''}>${e.name}　${e.adjustable ? ('音量 ' + e.volume + '%') : '（固定音量·不可调）'}</option>`
      ).join('');
    }
    if (!cur) { devEl.textContent = '没有可用的输出设备'; sl.disabled = true; return; }
    devEl.textContent = cur.name || '输出设备';
    sl.value = cur.volume;
    val.textContent = cur.volume + '%';
    sysVol.muted = cur.mute;
    muteBtn.textContent = cur.mute ? '🔕 取消静音' : '🔇 静音';
    sl.disabled = !cur.adjustable;
    muteBtn.disabled = !cur.adjustable;
    hint.textContent = cur.adjustable
      ? `控制的是上面选中的输出设备（与系统托盘音量同源）。总亮度作用于所有屏幕，不依赖 DDC。`
      : '这个端点（显示器音频）是固定音量，Windows 自己的滑块也调不动 → 想调这块屏的喇叭，用下面的「小米音量」滑块（走显示器自己的系统）；或在上面的「输出设备」里换成扬声器/耳机。';
  } catch (e) {
    card.style.display = '';
    devEl.textContent = '不可用';
    hint.textContent = '读取输出设备失败：' + errText(e);
  }
}

// ===== 电池设备（Windows：Battery 设备类枚举，内电池 / USB·无线键鼠 / 蓝牙） =====
function battIcon(conn) {
  if (conn === '内置') return '💻';
  if (conn === '蓝牙') return '🎧';
  if (conn === 'USB/无线' || conn === 'USB') return '🖱️';
  return '🔋';
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
let hdrStat = { sup: 0, on: 0, total: 0 };
async function loadHdrStatus() {
  const btn = document.getElementById('hdr-sync');
  if (!btn) return;
  try {
    const raw = String(await invoke('hdr_states') || '');
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
    if (!sup) txt = 'HDR：本机不支持';
    else if (on === sup) txt = 'HDR：开';
    else if (on === 0) txt = 'HDR：关';
    else txt = `HDR：混合（${on}/${sup} 开）`;
    btn.textContent = '🌈 ' + txt;
    btn.title = '点一下把所有屏的 HDR 设成一致';
  } catch (e) {
    btn.textContent = '🌈 HDR：读取失败';
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
  try {
    const hk = await invoke('hotkey_start');
    el.innerHTML = `全局快捷键：<span style="font-family:Consolas,monospace;background:rgba(255,255,255,.10);border:1px solid rgba(255,255,255,.12);border-radius:5px;padding:1px 6px">${hk}</span> 切换「全部屏幕待机 / 唤醒」 ｜ <span style="font-family:Consolas,monospace;background:rgba(255,255,255,.10);border:1px solid rgba(255,255,255,.12);border-radius:5px;padding:1px 6px">Ctrl+Alt+R</span> 还原被铺满的窗口（铺满后界面被盖住也能用）`;
  } catch (e) { el.textContent = '全局快捷键：' + errText(e); }
})();

// 铺满任意前台窗口
// 铺满方式：1 = 以主屏为基准（默认，主屏不会被裁）；0 = 铺满整块桌面包围盒
function spanMode() {
  const el = document.getElementById('span-mode');
  return el && el.value === '0' ? 0 : 1;
}

document.getElementById('span-fg').onclick = async () => {
  try {
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
    setStatus('✅ ' + msg + ' —— 注意：窗口现在不会立刻变小，Windows 限制必须先注销或重新登录（原值已备份，「还原缩放」可回退）');
  } catch (e) { setStatus('对齐失败：' + errText(e), false); }
};
document.getElementById('dpi-restore').onclick = async () => {
  try {
    const msg = await invoke('match_dpi_restore');
    setStatus('✅ ' + msg + ' —— 同样需要注销或重新登录才生效');
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
    if (!sec) { btn.textContent = '🔄 副屏方向：未检测到副屏'; return; }
    // 后端返回的是 resolution 字符串（如 "2400x3840"），不是 w/h 字段
    let W = Number(sec.w != null ? sec.w : sec.width);
    let H = Number(sec.h != null ? sec.h : sec.height);
    if (!W || !H) {
      const m = String(sec.resolution || '').match(/^(\d+)\s*x\s*(\d+)/i);
      if (m) { W = Number(m[1]); H = Number(m[2]); }
    }
    const portrait = H > W;
    btn.textContent = `🔄 副屏方向：${portrait ? '竖屏' : '横屏'}${W ? `（${W}x${H}）` : ''}（点击切换）`;
    btn.dataset.portrait = portrait ? '1' : '0';
  } catch (e) { btn.textContent = '🔄 副屏方向：读取失败'; }
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
    for (const line of raw.split('\n')) {
      const p = line.split('|');
      if (p[0] === 'W' && p.length >= 4) rows.push({ hwnd: p[1], size: p[2], title: p.slice(3).join('|') });
    }
    sel.innerHTML = rows.length
      ? rows.map(r => `<option value="${r.hwnd}">${r.title}　(${r.size})</option>`).join('')
      : '<option value="">（没有可铺满的窗口）</option>';
    setStatus(`找到 ${rows.length} 个可铺满的窗口，选中后点「铺满选中窗口」`);
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
    setStatus('正在铺满…');
    const title = await invoke('span_window', { hwnd, mode: spanMode() });
    setStatus(`已铺满：「${title}」。还原：点「↩ 还原」或按 Ctrl+Alt+R`);
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
  const hint = document.getElementById('mitv-hint');
  if (!row) return;
  try {
    const ok = await invoke('mitv_available');
    if (!ok) { row.style.display = 'none'; if (hint) hint.style.display = 'none'; return; }
    const v = await invoke('mitv_volume_get');
    row.style.display = '';
    if (hint) hint.style.display = '';
    document.getElementById('mitv-vol').value = v;
    document.getElementById('mitv-vol-val').textContent = v + '%';
  } catch (e) {
    row.style.display = 'none';
    if (hint) hint.style.display = 'none';
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
  const dirEl = document.getElementById('diag-dir');
  if (dirEl) {
    (async () => {
      try {
        const p = await invoke('diag_log_path');
        dirEl.textContent = '日志目录：' + p + '（每次检测一个 diag-…log，并追加一行到 diag-history.log）';
      } catch (e) { dirEl.textContent = '日志目录：%APPDATA%\\Screenguard\\logs'; }
    })();
  }
  if (open) open.onclick = async () => {
    try { await invoke('open_log_dir'); setStatus('已打开日志文件夹'); }
    catch (e) { setStatus('打开失败：' + errText(e), false); }
  };
  const fix = document.getElementById('diag-fix');
  if (fix) fix.onclick = async () => {
    fix.disabled = true;
    pre.textContent = '正在修复…（重协商显示链路 / 亮度复位 / 切换到可调音频端点 / 补齐恢复基线 / 还原被铺满的窗口，然后自动复查）';
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
    pre.textContent = '正在检测…（依次读取显示器 / 事件日志 / HDR / 音频 / 小米屏 / 电池 / 基线，约几秒）';
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
    const ok = confirm('把两块屏切到「竖向高度一致」的分辨率组合？\n\n· 切换时屏幕会黑一下（正常）\n· 已自动记录原始分辨率：关闭双屏铺满 / 退出软件 / 一键恢复默认 都会自动还原成 4K\n· 若切换失败会自动回滚\n\n继续吗？');
    if (!ok) return;
    ap.disabled = true;
    if (out) out.textContent = '正在试算并切换…（屏幕可能短暂黑屏）';
    try {
      const r = await invoke('align_apply');
      if (out) out.textContent = String(r);
      setStatus('双屏已对齐（关闭双屏铺满或退出软件会自动还原 4K）');
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
      sizes.textContent = '已记录尺寸：' + (arr.length ? arr.map(s => s.replace('|', ' → ') + ' 英寸').join('　') : '（无）')
        + '　｜　EDID 会上报尺寸的屏不用填；没上报的屏请填「厂商代码 + 对角线英寸」。厂商代码见体检结果里的括号。';
    } catch (e) { sizes.textContent = '已记录尺寸：读取失败'; }
  }
  if (btn) btn.onclick = async () => {
    btn.disabled = true;
    pre.textContent = '正在体检…（读 EDID 物理尺寸 + 每屏全部可用分辨率，约几秒）';
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
  paintSizes();
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
  } catch (e) { /* 预览模式：静默 */ }
  await refreshDisplays();
})();
