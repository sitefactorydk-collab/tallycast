import { invoke, listen, currentWindow, isTauri } from './tauri.js';
import { icon, hydrateIcons } from './icons.js';

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
    hint.textContent = 'Checking your graphics card…';
    $('#engine-status').textContent = 'Checking your graphics card…';
    return;
  }
  const chosen = state.settings.encoder || 'auto';
  const auto = view.encoders.find((e) => e.id === view.plan.encoder);
  sel.innerHTML = '';
  const add = (value, text, disabled = false) => {
    const o = document.createElement('option');
    o.value = value;
    o.textContent = text;
    o.disabled = disabled;
    sel.append(o);
  };
  add('auto', chosen === 'auto' && auto ? `Automatic – ${auto.label.split(' ')[0]}` : 'Automatic');
  view.encoders.forEach((e) => add(e.id, e.available ? e.label : `${e.label} – not found`, !e.available));
  sel.value = view.encoders.some((e) => e.id === chosen && e.available) ? chosen : 'auto';
  sel.disabled = false;
  const gpu = view.plan.encoder !== 'x264';
  hint.textContent = view.plan.direct
    ? 'Graphics card, captured and encoded on the GPU'
    : gpu ? 'Graphics card encodes the video' : 'Processor encodes the video';
  setChecked($('[aria-label="Screen capture"]'), 'data-capture', state.settings.capture || 'auto');
  const found = view.encoders.filter((e) => e.available && e.id !== 'x264').map((e) => e.label);
  $('#engine-status').textContent =
    `Recording uses ${view.planText}. ` +
    (found.length ? `Found: ${found.join(', ')}.` : 'No graphics-card encoder was found.') +
    (view.ddagrab ? '' : ' Desktop Duplication is not available.');
}

async function loadEngines(force = false) {
  if (state.env?.os !== 'windows') return renderEngines();
  if (force) {
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
  $('#ffmpeg-banner').hidden = Boolean(state.env?.ffmpegPath);
}

async function startCapture() {
  // Release the camera so the floating bubble can open it.
  stopCamPreview();
  clearTimeout(saveTimer);
  await invoke('save_settings', { settings: state.settings }).catch(() => {});
  await invoke('begin_capture', { mode: state.mode, monitor: state.monitor });
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
    const li = document.createElement('li');
    li.className = `rec${r.id === newId ? ' is-new' : ''}`;
    li.innerHTML = `
      <button class="thumb" aria-label="Play ${escapeHtml(r.title)}">
        ${r.thumbnail ? `<img src="${r.thumbnail}" alt="" draggable="false" />` : `<span class="thumb-empty">${icon('film')}</span>`}
        <span class="thumb-play">${icon('play')}</span>
        <span class="thumb-time tabular">${formatDuration(r.durationSec)}</span>
      </button>
      <div class="rec-info">
        <p class="rec-title" title="${escapeHtml(r.path)}">${escapeHtml(r.title)}</p>
        <p class="rec-meta tabular">
          <span>${formatWhen(r.createdMs)}</span>
          <span>${r.width} × ${r.height}</span>
          <span>${formatSize(r.sizeBytes)}</span>
        </p>
      </div>
      <div class="rec-actions">
        <button class="icon-btn" data-act="edit" title="Trim and cut" aria-label="Trim and cut">${icon('scissors')}</button>
        <button class="icon-btn" data-act="open" title="Play" aria-label="Play">${icon('play')}</button>
        <button class="icon-btn" data-act="reveal" title="Show in folder" aria-label="Show in folder">${icon('reveal')}</button>
        <button class="icon-btn danger" data-act="delete" title="Delete" aria-label="Delete">${icon('trash')}</button>
      </div>`;
    const run = (cmd) => invoke(cmd, { id: r.id }).catch((e) => toast(errorText(e), 'error'));
    li.querySelector('.thumb').onclick = () => run('open_recording');
    li.querySelector('[data-act="open"]').onclick = () => run('open_recording');
    li.querySelector('[data-act="edit"]').onclick = () => run('open_editor');
    li.querySelector('[data-act="reveal"]').onclick = () => run('reveal_recording');
    const del = li.querySelector('[data-act="delete"]');
    del.onclick = async () => {
      // Two-step delete: the first click arms the button for a few seconds.
      if (!del.classList.contains('armed')) {
        del.classList.add('armed');
        del.title = 'Click again to delete the video file';
        del.setAttribute('aria-label', 'Click again to delete the video file');
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
  $('#open-folder').onclick = () => invoke('open_output_dir').catch((e) => toast(errorText(e), 'error'));
}

// ───────────────────────────── start-up ─────────────────────────────

async function init() {
  hydrateIcons();
  wireTitlebar();
  wireAudio();
  wireWebcam();
  wireVideo();
  wireSettings();
  wireEngines();
  $('#record-btn').onclick = () => startCapture().catch((e) => toast(errorText(e), 'error'));

  state.settings = await invoke('get_settings');
  state.mode = state.settings.lastMode === 'area' ? 'area' : 'screen';
  state.monitor = state.settings.lastMonitor || 0;

  await refreshEnvironment();
  $('#hotkey-record').textContent = state.env.recordHotkey;
  $('#hotkey-pause').textContent = state.env.pauseHotkey;
  renderMode();
  renderAudio();
  renderVideo();
  loadEngines();
  if (state.env.wayland) {
    toast('Recording on Wayland is not supported yet. Log in with an X11 session to record.', 'error');
  }
  await refreshLibrary();
  if (state.settings.webcamEnabled) setWebcam(true);

  await listen('recording-saved', (rec) => {
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
}

init().catch((e) => toast(`Tallycast could not start: ${errorText(e)}`, 'error'));
