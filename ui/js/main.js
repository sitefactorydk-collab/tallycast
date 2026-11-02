import { invoke, listen, currentWindow, isTauri } from './tauri.js';
import { icon, hydrateIcons } from './icons.js';
import { mountFx, HALO_SIZES, SPAN, HALO_COLORS, CLICK_COLORS } from './cursorfx.js';

const $ = (sel) => document.querySelector(sel);

const state = {
  env: null,
  settings: null,
  recordings: [],
  mode: 'screen',
  monitor: 0,
  camStream: null,
};

// ───────────────────────────── helpers ─────────────────────────────

function formatDuration(sec) {
  const s = Math.max(0, Math.round(sec));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${r}` : `${m}:${r}`;
}

function formatSize(bytes) {
  if (!bytes) return '';
  const mb = bytes / 1_000_000;
  return mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : mb >= 10 ? `${Math.round(mb)} MB` : `${mb.toFixed(1)} MB`;
}

function formatWhen(ms) {
  const d = new Date(ms);
  const today = new Date();
  const yesterday = new Date(Date.now() - 86_400_000);
  const time = d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
  if (d.toDateString() === today.toDateString()) return `Today, ${time}`;
  if (d.toDateString() === yesterday.toDateString()) return `Yesterday, ${time}`;
  return `${d.toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: d.getFullYear() === today.getFullYear() ? undefined : 'numeric' })}, ${time}`;
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
}

function errorText(e) {
  return typeof e === 'string' ? e : e?.message || String(e);
}

let saveTimer = 0;
function updateSettings(patch) {
  Object.assign(state.settings, patch);
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    invoke('save_settings', { settings: state.settings }).catch((e) => toast(errorText(e), 'error'));
  }, 250);
}

function setChecked(group, attr, value) {
  group.querySelectorAll(`[${attr}]`).forEach((b) => {
    b.setAttribute('aria-checked', String(b.getAttribute(attr) === String(value)));
  });
}

// ───────────────────────────── toasts ─────────────────────────────

function toast(message, kind = 'info', action = null) {
  const el = document.createElement('div');
  el.className = `toast ${kind}`;
  el.setAttribute('role', kind === 'error' ? 'alert' : 'status');
  const ic = kind === 'error' ? 'alert' : kind === 'ok' ? 'check' : 'film';
  el.innerHTML = `${icon(ic)}<div class="toast-body"></div>`;
  el.querySelector('.toast-body').textContent = message;
  if (action) {
    const b = document.createElement('button');
    b.className = 'toast-action';
    b.textContent = action.label;
    b.onclick = () => { action.run(); el.remove(); };
    el.append(b);
  }
  const close = document.createElement('button');
  close.className = 'toast-close';
  close.setAttribute('aria-label', 'Dismiss');
  close.innerHTML = icon('close');
  close.onclick = () => el.remove();
  el.append(close);
  $('#toasts').append(el);
  setTimeout(() => el.remove(), kind === 'error' ? 14000 : 6000);
}

// ───────────────────────────── title bar ─────────────────────────────

function wireTitlebar() {
  const win = currentWindow();
  $('#btn-min').onclick = () => win?.minimize();
  $('#btn-max').onclick = () => win?.toggleMaximize();
  $('#btn-close').onclick = () => win?.close();
  $('#btn-settings').onclick = openSettings;
}

// ───────────────────────────── capture source ─────────────────────────────

function renderMode() {
  setChecked($('.group-head .segmented'), 'data-mode', state.mode);
  $('#display-map').dataset.mode = state.mode;
  $('#record-label').textContent = state.mode === 'area' ? 'Select area and record' : 'Record full screen';
  renderCaption();
}

function renderDisplays() {
  const map = $('#display-map');
  const mons = state.env.monitors;
  map.innerHTML = '';
  if (!mons.length) {
    map.innerHTML = '<p class="display-caption" style="padding:16px">No displays were detected.</p>';
    return;
  }
  // Fit the real desktop layout into the map, to scale.
  const minX = Math.min(...mons.map((m) => m.x));
  const minY = Math.min(...mons.map((m) => m.y));
  const maxX = Math.max(...mons.map((m) => m.x + m.width));
  const maxY = Math.max(...mons.map((m) => m.y + m.height));
  const box = map.getBoundingClientRect();
  const pad = 22;
  const scale = Math.min((box.width - pad * 2) / (maxX - minX), (box.height - pad * 2) / (maxY - minY));
  const offX = (box.width - (maxX - minX) * scale) / 2;
  const offY = (box.height - (maxY - minY) * scale) / 2;
  const gap = mons.length > 1 ? 3 : 0;

  mons.forEach((m, i) => {
    const b = document.createElement('button');
    b.className = 'monitor';
    b.setAttribute('role', 'radio');
    b.setAttribute('aria-checked', String(m.index === state.monitor));
    b.setAttribute('aria-label', `Display ${i + 1}, ${m.width} by ${m.height}${m.primary ? ', main display' : ''}`);
    b.style.left = `${offX + (m.x - minX) * scale + gap}px`;
    b.style.top = `${offY + (m.y - minY) * scale + gap}px`;
    b.style.width = `${m.width * scale - gap * 2}px`;
    b.style.height = `${m.height * scale - gap * 2}px`;
    b.innerHTML = `<span class="monitor-num">${i + 1}</span><span class="monitor-res tabular">${m.width} × ${m.height}</span><span class="monitor-area"></span>`;
    b.onclick = () => {
      state.monitor = m.index;
      updateSettings({ lastMonitor: m.index });
      renderDisplays();
      renderCaption();
    };
    map.append(b);
  });
}

function renderCaption() {
  const mons = state.env?.monitors || [];
  const i = mons.findIndex((m) => m.index === state.monitor);
  const m = mons[i];
  if (!m) return;
  const which = mons.length > 1 ? `Display ${i + 1}${m.primary ? ' (main)' : ''}` : 'Your screen';
  const scale = m.scale && m.scale !== 1 ? `, scaled ${Math.round(m.scale * 100)}%` : '';
  $('#display-caption').textContent =
    state.mode === 'area'
      ? `${which}: you'll drag out the area after pressing Record.`
      : `${which}: ${m.width} × ${m.height}${scale}.`;
}

// ───────────────────────────── audio ─────────────────────────────

function renderAudio() {
  const { env, settings } = state;
  const sel = $('#mic-select');
  sel.innerHTML = '';
  const def = document.createElement('option');
  def.value = '';
  def.textContent = env.defaultMic ? `Default (${env.defaultMic})` : 'System default';
  sel.append(def);
  env.micDevices.forEach((name) => {
    const o = document.createElement('option');
    o.value = name;
    o.textContent = name;
    sel.append(o);
  });
  sel.value = settings.micDevice && env.micDevices.includes(settings.micDevice) ? settings.micDevice : '';
  sel.disabled = !settings.micEnabled;
  $('#mic-toggle').checked = settings.micEnabled;

  const sys = $('#sys-toggle');
  sys.disabled = !env.systemAudioSupported;
  sys.checked = env.systemAudioSupported && settings.systemAudio;
  const hints = {
    windows: 'No speakers or headphones found. Connect one to record computer sound',
    macos: 'Needs a virtual audio device such as BlackHole on macOS',
  };
  $('#sys-hint').textContent = env.systemAudioSupported
    ? 'Everything you hear from your speakers'
    : hints[env.os] || 'Not available on Linux yet';

  const hasMic = env.micDevices.length > 0;
  $('#mic-toggle').disabled = !hasMic;
  if (!hasMic) {
    $('#mic-toggle').checked = false;
    sel.disabled = true;
    def.textContent = 'No microphone found';
  }
}

function wireAudio() {
  $('#mic-toggle').onchange = (e) => {
    updateSettings({ micEnabled: e.target.checked });
    $('#mic-select').disabled = !e.target.checked;
  };
  $('#mic-select').onchange = (e) => updateSettings({ micDevice: e.target.value || null });
  $('#sys-toggle').onchange = (e) => updateSettings({ systemAudio: e.target.checked });
}

// ───────────────────────────── webcam ─────────────────────────────

async function startCamPreview() {
  stopCamPreview();
  const deviceId = state.settings.webcamDevice;
  try {
    state.camStream = await navigator.mediaDevices.getUserMedia({
      video: deviceId ? { deviceId: { exact: deviceId } } : true,
      audio: false,
    });
  } catch (e) {
    if (deviceId) {
      // The saved camera may be unplugged: fall back to any camera.
      updateSettings({ webcamDevice: null });
      return startCamPreview();
    }
    throw e;
  }
  $('#cam-video').srcObject = state.camStream;
  $('#cam-preview').hidden = false;
  await listCameras();
}

function stopCamPreview() {
  state.camStream?.getTracks().forEach((t) => t.stop());
  state.camStream = null;
  $('#cam-video').srcObject = null;
}

async function listCameras() {
  const sel = $('#cam-select');
  const devices = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === 'videoinput');
  sel.innerHTML = '';
  devices.forEach((d, i) => {
    const o = document.createElement('option');
    o.value = d.deviceId;
    o.textContent = d.label || `Camera ${i + 1}`;
    sel.append(o);
  });
  const active = state.camStream?.getVideoTracks()[0]?.getSettings().deviceId;
  sel.value = state.settings.webcamDevice || active || devices[0]?.deviceId || '';
  sel.disabled = devices.length === 0;
}

async function setWebcam(enabled) {
  $('#cam-toggle').checked = enabled;
  if (!enabled) {
    stopCamPreview();
    $('#cam-preview').hidden = true;
    $('#cam-select').disabled = true;
    updateSettings({ webcamEnabled: false });
    return;
  }
  try {
    await startCamPreview();
    updateSettings({ webcamEnabled: true });
  } catch (e) {
    $('#cam-toggle').checked = false;
    $('#cam-preview').hidden = true;
    updateSettings({ webcamEnabled: false });
    const denied = e?.name === 'NotAllowedError';
    toast(denied
      ? 'Tallycast is not allowed to use the camera. Allow camera access in Windows Settings › Privacy & security › Camera, then try again.'
      : `The camera could not be started (${errorText(e)}).`, 'error');
  }
}

function wireWebcam() {
  $('#cam-toggle').onchange = (e) => setWebcam(e.target.checked);
  $('#cam-select').onchange = async (e) => {
    updateSettings({ webcamDevice: e.target.value || null });
    try { await startCamPreview(); } catch (err) { toast(errorText(err), 'error'); }
  };
}

// ───────────────────────────── video ─────────────────────────────

function renderVideo() {
  setChecked($('[aria-label="Frame rate"]'), 'data-fps', state.settings.fps);
  $('#quality-select').value = state.settings.quality;
}

function wireVideo() {
  document.querySelectorAll('[data-fps]').forEach((b) => {
    b.onclick = () => { updateSettings({ fps: Number(b.dataset.fps) }); renderVideo(); };
  });
  $('#quality-select').onchange = (e) => updateSettings({ quality: e.target.value });
  document.querySelectorAll('[data-mode]').forEach((b) => {
    b.onclick = () => { state.mode = b.dataset.mode; updateSettings({ lastMode: state.mode }); renderMode(); };
  });
}

// ───────────────────────────── mouse pointer effects ─────────────────────────────

let pointerFx = null;

function renderSwatches(el, colors, current, enabled, pick) {
  const cur = String(current || '').toLowerCase();
  el.innerHTML = colors
    .map((c) => `<button type="button" role="radio" style="--c:${c.value}" title="${c.name}" aria-label="${c.name}" aria-checked="${c.value === cur}" data-color="${c.value}"></button>`)
    .join('');
  el.classList.toggle('off', !enabled);
  el.querySelectorAll('button').forEach((b) => { b.onclick = () => pick(b.dataset.color); });
}

function renderPointer() {
  const s = state.settings;
  // The effects follow the pointer with Windows calls; other systems don't have them yet.
  $('#pointer-group').hidden = state.env?.os !== 'windows';
  $('#halo-toggle').checked = Boolean(s.cursorHalo);
  $('#click-toggle').checked = Boolean(s.clickEffects);
  // Picking a colour also switches that effect on.
  renderSwatches($('#halo-colors'), HALO_COLORS, s.haloColor, s.cursorHalo, (c) => {
    updateSettings({ haloColor: c, cursorHalo: true });
    renderPointer();
  });
  renderSwatches($('#click-colors'), CLICK_COLORS, s.clickColor, s.clickEffects, (c) => {
    updateSettings({ clickColor: c, clickEffects: true });
    renderPointer();
  });
  const size = HALO_SIZES[s.cursorFxSize] ? s.cursorFxSize : 'm';
  setChecked($('.fx-size'), 'data-fxsize', size);
  $('#fx-pad').hidden = !(s.cursorHalo || s.clickEffects);
  const box = $('#fx-preview');
  box.style.width = box.style.height = `${HALO_SIZES[size] * SPAN}px`;
  pointerFx.set({
    halo: Boolean(s.cursorHalo),
    click: Boolean(s.clickEffects),
    haloColor: s.haloColor,
    clickColor: s.clickColor,
  });
}

function wirePointer() {
  const pad = $('#fx-pad');
  const box = $('#fx-preview');
  pointerFx = mountFx(box);
  $('#halo-toggle').onchange = (e) => { updateSettings({ cursorHalo: e.target.checked }); renderPointer(); };
  $('#click-toggle').onchange = (e) => { updateSettings({ clickEffects: e.target.checked }); renderPointer(); };
  document.querySelectorAll('[data-fxsize]').forEach((b) => {
    b.onclick = () => { updateSettings({ cursorFxSize: b.dataset.fxsize }); renderPointer(); };
  });

  // The preview pad shows the effect under the real pointer, as it will look in a video.
  const place = (e) => {
    const r = pad.getBoundingClientRect();
    const half = box.offsetWidth / 2;
    box.style.transform = `translate(${e.clientX - r.left - half}px, ${e.clientY - r.top - half}px)`;
    pad.classList.add('live');
  };
  pad.addEventListener('pointermove', place);
  pad.addEventListener('pointerleave', () => { pad.classList.remove('live'); pointerFx.up(); });
  pad.addEventListener('pointerdown', (e) => {
    if (e.target.closest('.fx-size')) return;
    place(e);
    pointerFx.down(e.button === 2 ? 'right' : 'left');
  });
  pad.addEventListener('pointerup', () => pointerFx.up());
  pad.addEventListener('contextmenu', (e) => e.preventDefault());
}

// ───────────────────────────── video engine ─────────────────────────────

function renderEngines() {
  const view = state.engines;
  const supported = state.env?.os === 'windows';
  $('#encoder-row').hidden = !supported;
  $('#capture-field').hidden = !supported;
  if (!supported) return;
  const sel = $('#encoder-select');
  const hint = $('#encoder-hint');
  if (!view) {
    sel.disabled = true;
    hint.textContent = 'Testing your graphics card…';
    $('#engine-status').textContent = 'Testing your graphics card…';
    return;
  }
  // The processor is the default; graphics-card encoders are opt-in.
  const chosen = state.settings.encoder || 'auto';
  sel.innerHTML = '';
  const add = (value, text, disabled = false) => {
    const o = document.createElement('option');
    o.value = value;
    o.textContent = text;
    o.disabled = disabled;
    sel.append(o);
  };
  add('auto', 'Processor (x264)');
  view.encoders
    .filter((e) => e.id !== 'x264')
    .forEach((e) => add(e.id, e.available ? e.label : `${e.label} – not found`, !e.available));
  sel.value = view.encoders.some((e) => e.id === chosen && e.id !== 'x264' && e.available) ? chosen : 'auto';
  sel.disabled = false;
  const gpu = view.plan.encoder !== 'x264';
  hint.textContent = view.plan.direct
    ? 'Graphics card, captured and encoded on the GPU'
    : gpu ? 'Graphics card encodes the video' : 'Processor encodes the video';
  setChecked($('[aria-label="Screen capture"]'), 'data-capture', state.settings.capture || 'auto');
  const found = view.encoders.filter((e) => e.available && e.id !== 'x264').map((e) => e.label);
  $('#engine-status').textContent =
    `Recording uses ${view.planText}. ` +
    (!view.tested
      ? 'The graphics card is tested when you choose one of its encoders.'
      : (found.length ? `Found: ${found.join(', ')}.` : 'No graphics-card encoder was found.') +
        (view.ddagrab ? '' : ' Desktop Duplication is not available.'));
}

async function loadEngines(force = false) {
  if (state.env?.os !== 'windows') return renderEngines();
  // Testing runs FFmpeg for a few seconds; say so while it happens.
  const gpu = !['auto', 'x264'].includes(state.settings.encoder || 'auto') || state.settings.capture === 'ddagrab';
  if (force || (gpu && !state.engines?.tested)) {
    state.engines = null;
    renderEngines();
  }
  try {
    state.engines = await invoke('get_engines', { force });
  } catch (e) {
    state.engines = null;
    $('#encoder-hint').textContent = errorText(e);
    return;
  }
  renderEngines();
}

async function saveNow() {
  clearTimeout(saveTimer);
  await invoke('save_settings', { settings: state.settings }).catch((e) => toast(errorText(e), 'error'));
}

function wireEngines() {
  $('#encoder-select').onchange = async (e) => {
    state.settings.encoder = e.target.value;
    await saveNow();
    loadEngines();
  };
  document.querySelectorAll('[data-capture]').forEach((b) => {
    b.onclick = async () => {
      state.settings.capture = b.dataset.capture;
      renderEngines();
      await saveNow();
      loadEngines();
    };
  });
  $('#engine-retest').onclick = () => loadEngines(true);
}

// ───────────────────────────── recording ─────────────────────────────

function renderRecordButton() {
  const ok = Boolean(state.env?.ffmpegPath) && !state.env?.wayland;
  $('#record-btn').disabled = !ok;
  $('#shot-btn').disabled = !ok;
  $('#ffmpeg-banner').hidden = Boolean(state.env?.ffmpegPath);
}

async function startCapture() {
  // Release the camera so the floating bubble can open it.
  stopCamPreview();
  clearTimeout(saveTimer);
  await invoke('save_settings', { settings: state.settings }).catch(() => {});
  await invoke('begin_capture', { mode: state.mode, monitor: state.monitor });
}

async function startScreenshot() {
  clearTimeout(saveTimer);
  await invoke('save_settings', { settings: state.settings }).catch(() => {});
  await invoke('begin_screenshot', { mode: state.mode, monitor: state.monitor });
}

/** Puts a screenshot's PNG (with its marks) on the clipboard. */
async function copyImage(r, button) {
  try {
    // Hand the clipboard a promise right away, while the click still counts as a user
    // gesture; the PNG is fetched from the local media server meanwhile.
    const png = invoke('recording_url', { id: r.id })
      .then((url) => fetch(url))
      .then((res) => {
        if (!res.ok) throw new Error(`the file could not be read (${res.status})`);
        return res.blob();
      })
      .then((blob) => (blob.type === 'image/png' ? blob : new Blob([blob], { type: 'image/png' })));
    await navigator.clipboard.write([new ClipboardItem({ 'image/png': png })]);
    toast('Copied. Paste it anywhere with Ctrl+V.', 'ok');
    button.classList.add('done');
    setTimeout(() => button.classList.remove('done'), 1500);
  } catch (e) {
    toast(`Could not copy the image: ${errorText(e)}`, 'error');
  }
}

// ───────────────────────────── library ─────────────────────────────

function renderLibrary(newId = null) {
  const list = $('#rec-list');
  const recs = state.recordings;
  $('#rec-count').textContent = recs.length ? recs.length : '';
  $('#empty').hidden = recs.length > 0;
  list.hidden = recs.length === 0;
  list.innerHTML = '';
  recs.forEach((r) => {
    const image = r.kind === 'image';
    const li = document.createElement('li');
    li.className = `rec${r.id === newId ? ' is-new' : ''}${image ? ' is-image' : ''}`;
    li.innerHTML = `
      <button class="thumb" aria-label="${image ? 'Annotate' : 'Play'} ${escapeHtml(r.title)}">
        ${r.thumbnail ? `<img src="${r.thumbnail}" alt="" draggable="false" />` : `<span class="thumb-empty">${icon(image ? 'shot' : 'film')}</span>`}
        <span class="thumb-play">${icon(image ? 'annotate' : 'play')}</span>
        <span class="thumb-time tabular">${image ? 'PNG' : formatDuration(r.durationSec)}</span>
      </button>
      <div class="rec-info">
        <p class="rec-title" title="${escapeHtml(r.path)}">${escapeHtml(r.title)}</p>
        <p class="rec-meta tabular">
          <span>${formatWhen(r.createdMs)}</span>
          <span>${r.width} × ${r.height}</span>
          <span>${formatSize(r.sizeBytes)}</span>
        </p>
      </div>
      <div class="rec-side">
      <div class="rec-actions">
        ${image
          ? `<button class="icon-btn" data-act="edit" title="Annotate" aria-label="Annotate">${icon('annotate')}</button>
        <button class="icon-btn" data-act="open" title="Open in your image viewer" aria-label="Open in your image viewer">${icon('open')}</button>`
          : `<button class="icon-btn" data-act="edit" title="Trim and cut" aria-label="Trim and cut">${icon('scissors')}</button>
        <button class="icon-btn" data-act="open" title="Play" aria-label="Play">${icon('play')}</button>`}
        <button class="icon-btn" data-act="reveal" title="Show in folder" aria-label="Show in folder">${icon('reveal')}</button>
        <button class="icon-btn danger" data-act="delete" title="Delete" aria-label="Delete">${icon('trash')}</button>
      </div>
      ${image ? `<button class="btn btn-quiet rec-copy" data-act="copy" title="Copy the image to the clipboard">${icon('copy')}Copy</button>` : ''}
      </div>`;
    const run = (cmd) => invoke(cmd, { id: r.id }).catch((e) => toast(errorText(e), 'error'));
    li.querySelector('.thumb').onclick = () => run(image ? 'open_shot' : 'open_recording');
    li.querySelector('[data-act="open"]').onclick = () => run('open_recording');
    li.querySelector('[data-act="edit"]').onclick = () => run(image ? 'open_shot' : 'open_editor');
    li.querySelector('[data-act="reveal"]').onclick = () => run('reveal_recording');
    const copyBtn = li.querySelector('[data-act="copy"]');
    if (copyBtn) copyBtn.onclick = () => copyImage(r, copyBtn);
    const del = li.querySelector('[data-act="delete"]');
    del.onclick = async () => {
      // Two-step delete: the first click arms the button for a few seconds.
      if (!del.classList.contains('armed')) {
        del.classList.add('armed');
        const what = image ? 'Click again to delete the image file' : 'Click again to delete the video file';
        del.title = what;
        del.setAttribute('aria-label', what);
        setTimeout(() => { del.classList.remove('armed'); del.title = 'Delete'; del.setAttribute('aria-label', 'Delete'); }, 3000);
        return;
      }
      try {
        await invoke('delete_recording', { id: r.id });
        state.recordings = state.recordings.filter((x) => x.id !== r.id);
        renderLibrary();
        toast('Recording deleted.');
      } catch (e) {
        toast(errorText(e), 'error');
      }
    };
    list.append(li);
  });
}

async function refreshLibrary() {
  state.recordings = (await invoke('list_recordings')) || [];
  renderLibrary();
}

// ───────────────────────────── settings sheet ─────────────────────────────

function renderSettingsSheet() {
  const { env, settings } = state;
  $('#out-dir').textContent = settings.outputDir || env.outputDir;
  setChecked($('[aria-label="Countdown"]'), 'data-countdown', settings.countdown);
  $('#cursor-toggle').checked = settings.showCursor;
  $('#ffmpeg-path').textContent = env.ffmpegPath || 'Not found';
  $('#ffmpeg-version').textContent = env.ffmpegVersion || 'Install FFmpeg with: winget install Gyan.FFmpeg';
  $('#version-line').textContent = env.appVersion ? `Tallycast ${env.appVersion}` : '';
}

function openSettings() {
  renderSettingsSheet();
  $('#settings').showModal();
}

async function refreshEnvironment() {
  state.env = await invoke('get_environment');
  if (!state.env.monitors.some((m) => m.index === state.monitor)) {
    state.monitor = (state.env.monitors.find((m) => m.primary) || state.env.monitors[0] || { index: 0 }).index;
  }
  renderDisplays();
  renderCaption();
  renderRecordButton();
}

async function chooseFfmpeg() {
  const path = await invoke('pick_ffmpeg');
  if (!path) return;
  updateSettings({ ffmpegPath: path });
  clearTimeout(saveTimer);
  await invoke('save_settings', { settings: state.settings });
  await refreshEnvironment();
  renderSettingsSheet();
  toast(state.env.ffmpegPath ? 'FFmpeg found. You can record now.' : 'That file is not FFmpeg.', state.env.ffmpegPath ? 'ok' : 'error');
}

function wireSettings() {
  $('#pick-dir').onclick = async () => {
    const dir = await invoke('pick_folder');
    if (!dir) return;
    updateSettings({ outputDir: dir });
    renderSettingsSheet();
  };
  document.querySelectorAll('[data-countdown]').forEach((b) => {
    b.onclick = () => { updateSettings({ countdown: Number(b.dataset.countdown) }); renderSettingsSheet(); };
  });
  $('#cursor-toggle').onchange = (e) => updateSettings({ showCursor: e.target.checked });
  $('#pick-ffmpeg').onclick = chooseFfmpeg;
  $('#banner-locate').onclick = chooseFfmpeg;
  $('#auto-ffmpeg').onclick = async () => {
    updateSettings({ ffmpegPath: null });
    clearTimeout(saveTimer);
    await invoke('save_settings', { settings: state.settings });
    await refreshEnvironment();
    renderSettingsSheet();
  };
  $('#check-updates').onclick = () => checkForUpdates(true);
  $('#open-folder').onclick = () => invoke('open_output_dir').catch((e) => toast(errorText(e), 'error'));
}

// ───────────────────────────── updates ─────────────────────────────

const UPDATE_EVERY_MS = 12 * 60 * 60 * 1000;
const SKIP_KEY = 'tallycast.skipUpdate';

/** Compares "1.2.10" and "1.2.9" numerically; a pre-release suffix is ignored. */
function isNewer(latest, current) {
  const parts = (v) => String(v).replace(/^v/i, '').split('-')[0].split('.').map((n) => parseInt(n, 10) || 0);
  const a = parts(latest);
  const b = parts(current);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if ((a[i] || 0) !== (b[i] || 0)) return (a[i] || 0) > (b[i] || 0);
  }
  return false;
}

function skipped() {
  try { return localStorage.getItem(SKIP_KEY); } catch { return null; }
}

function openLink(url) {
  invoke('open_url', { url }).catch((e) => toast(errorText(e), 'error'));
}

let updating = false;

/** While an update runs, the banner shows only its progress. */
function setUpdateButtons(busy) {
  ['#update-download', '#update-notes', '#update-later'].forEach((sel) => { $(sel).hidden = busy; });
}

/**
 * Downloads, verifies and installs the update inside the app. Tallycast closes, the
 * installer runs with a small progress window, and Tallycast starts again. If that route
 * isn't available (e.g. the release has no signed update files), the installer opens in
 * the browser instead, as before.
 */
async function installUpdate(latest, setupUrl) {
  if (updating) return;
  updating = true;
  setUpdateButtons(true);
  const sub = $('#update-sub');
  const button = $('#update-download');
  $('#update-title').textContent = `Updating to Tallycast ${latest}…`;
  sub.textContent = 'Downloading…';
  const unlisten = [
    await listen('update-progress', ({ downloaded, total }) => {
      sub.textContent = total
        ? `Downloading… ${Math.min(100, Math.round((downloaded / total) * 100))} %`
        : `Downloading… ${(downloaded / 1e6).toFixed(1)} MB`;
    }),
    await listen('update-installing', () => {
      sub.textContent = 'Installing. Tallycast closes now and opens again when it is done.';
    }),
  ];
  const restore = () => {
    unlisten.forEach((off) => { try { off(); } catch { /* already gone */ } });
    updating = false;
    setUpdateButtons(false);
    $('#update-title').textContent = `Tallycast ${latest} is available.`;
    button.hidden = false;
  };
  try {
    const started = await invoke('install_update');
    if (!started) {
      restore();
      sub.textContent = 'This version can\'t be installed from inside Tallycast. The installer is downloading in your browser instead.';
      if (setupUrl) openLink(setupUrl);
    }
    // Otherwise Tallycast is closing for the installer.
  } catch (e) {
    restore();
    const msg = errorText(e);
    if (/close the editor/i.test(msg)) {
      sub.textContent = msg;
      return;
    }
    sub.textContent = setupUrl
      ? 'The update could not be installed from inside Tallycast, so the installer is downloading in your browser instead.'
      : 'The update could not be installed. Try again in a few minutes.';
    toast(`Update failed: ${msg}`, 'error');
    if (setupUrl) openLink(setupUrl);
  }
}

function showUpdate(release, latest) {
  if (updating) return;
  const current = state.env.appVersion;
  const setup = (release.assets || []).find((a) => /-setup\.exe$/i.test(a.name));
  $('#update-title').textContent = `Tallycast ${latest} is available.`;
  $('#update-sub').textContent = setup
    ? `You have ${current}. Update now: Tallycast restarts in about a minute, with your recordings and settings as they are.`
    : `You have ${current}. This version is still being built; try again in a few minutes.`;
  const button = $('#update-download');
  button.hidden = !setup;
  button.innerHTML = `${icon('download')}Update now`;
  button.onclick = () => installUpdate(latest, setup?.browser_download_url);
  $('#update-notes').onclick = () => openLink(release.html_url);
  $('#update-later').onclick = () => {
    try { localStorage.setItem(SKIP_KEY, latest); } catch { /* storage unavailable */ }
    $('#update-banner').hidden = true;
  };
  $('#update-banner').hidden = false;
}

/** Asks GitHub for the latest published release. `manual` reports every outcome. */
async function checkForUpdates(manual = false) {
  const { appVersion, repo } = state.env || {};
  if (!appVersion || !repo) return;
  const button = $('#check-updates');
  if (manual) { button.disabled = true; button.textContent = 'Checking…'; }
  try {
    const res = await fetch(`https://api.github.com/repos/${repo}/releases/latest`, {
      headers: { Accept: 'application/vnd.github+json' },
      cache: 'no-store',
    });
    if (res.status === 404) {
      if (manual) toast('No version has been published on GitHub yet.');
      return;
    }
    if (!res.ok) throw new Error(`GitHub answered ${res.status}`);
    const release = await res.json();
    const latest = String(release.tag_name || '').replace(/^v/i, '');
    if (latest && isNewer(latest, appVersion)) {
      if (manual || skipped() !== latest) showUpdate(release, latest);
      if (manual) $('#settings').close();
    } else if (manual) {
      toast(`You have the latest version (${appVersion}).`, 'ok');
    }
  } catch (e) {
    if (manual) toast(`Could not check for updates: ${errorText(e)}`, 'error');
  } finally {
    if (manual) { button.disabled = false; button.textContent = 'Check for updates'; }
  }
}

// ───────────────────────────── start-up ─────────────────────────────

async function init() {
  hydrateIcons();
  wireTitlebar();
  wireAudio();
  wireWebcam();
  wireVideo();
  wirePointer();
  wireSettings();
  wireEngines();
  $('#record-btn').onclick = () => startCapture().catch((e) => toast(errorText(e), 'error'));
  $('#shot-btn').onclick = () => startScreenshot().catch((e) => toast(errorText(e), 'error'));

  state.settings = await invoke('get_settings');
  state.mode = state.settings.lastMode === 'area' ? 'area' : 'screen';
  state.monitor = state.settings.lastMonitor || 0;

  await refreshEnvironment();
  $('#hotkey-record').textContent = state.env.recordHotkey;
  $('#hotkey-pause').textContent = state.env.pauseHotkey;
  $('#hotkey-shot').textContent = state.env.shotHotkey || 'Alt+Shift+S';
  renderMode();
  renderAudio();
  renderVideo();
  renderPointer();
  loadEngines();
  if (state.env.wayland) {
    toast('Recording on Wayland is not supported yet. Log in with an X11 session to record.', 'error');
  }
  await refreshLibrary();
  if (state.settings.webcamEnabled) setWebcam(true);

  await listen('recording-saved', (rec) => {
    const known = state.recordings.some((r) => r.id === rec.id);
    if (rec.kind === 'image') {
      // Screenshots open straight in Annotate and save often there: update quietly.
      state.recordings = known
        ? state.recordings.map((r) => (r.id === rec.id ? rec : r))
        : [rec, ...state.recordings];
      renderLibrary(known ? null : rec.id);
      return;
    }
    state.recordings = [rec, ...state.recordings.filter((r) => r.id !== rec.id)];
    renderLibrary(rec.id);
    // A fresh recording offers the editor; an exported edit offers its folder.
    const edited = rec.title.endsWith('(edited)') || / \(edited\) \(\d+\)$/.test(rec.title);
    toast(`Saved "${rec.title}"`, 'ok', edited
      ? { label: 'Show in folder', run: () => invoke('reveal_recording', { id: rec.id }) }
      : { label: 'Trim and cut', run: () => invoke('open_editor', { id: rec.id }).catch((e) => toast(errorText(e), 'error')) });
    (state.warnings || []).forEach((w) => toast(w, 'error'));
    state.warnings = [];
  });
  await listen('capture-error', (msg) => {
    state.warnings = [];
    toast(errorText(msg), 'error');
  });
  // Warnings arrive while this window is hidden; show them once the recording is saved.
  await listen('capture-warning', (msg) => { state.warnings = [...(state.warnings || []), errorText(msg)]; });

  // The window comes back after recording: refresh displays and the camera preview.
  let lastRefresh = Date.now();
  window.addEventListener('focus', () => {
    if (Date.now() - lastRefresh < 4000) return;
    lastRefresh = Date.now();
    // Devices may have been plugged in or removed while the window was in the background.
    refreshEnvironment().then(renderAudio).catch(() => {});
    if (state.settings.webcamEnabled && !state.camStream) startCamPreview().catch(() => {});
  });
  window.addEventListener('resize', () => renderDisplays());

  if (!isTauri) document.documentElement.dataset.preview = 'browser';

  // Quietly look for a newer release now and twice a day while the app stays open.
  checkForUpdates();
  setInterval(() => checkForUpdates(), UPDATE_EVERY_MS);
}

init().catch((e) => toast(`Tallycast could not start: ${errorText(e)}`, 'error'));
