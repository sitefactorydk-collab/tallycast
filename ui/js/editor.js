// Tallycast trim-and-cut editor.
//
// Times are seconds from the start of the recording. Every edit boundary is snapped
// to the start of a frame (frame k starts at videoStart + k / fps), and a span
// [a, b) keeps exactly the frames the player shows between a and b. The backend
// cuts with the same rule, so what you mark is what you export.

import { invoke, listen, currentWindow } from './tauri.js';
import { icon, hydrateIcons } from './icons.js';

const $ = (sel) => document.querySelector(sel);
const video = $('#video');
const scroller = $('#tl-scroll');
const content = $('#tl-content');
const canvas = $('#tl-canvas');
const ctx = canvas.getContext('2d');

// Timeline geometry (CSS pixels).
const PAD = 16;
const RULER_H = 26;
const STRIP_TOP = 30;
const STRIP_H = 58;
const WAVE_TOP = STRIP_TOP + STRIP_H + 6;
const WAVE_H = 38;
const LANES_BOTTOM = WAVE_TOP + WAVE_H;
const SNAP_PX = 7;

const S = {
  id: null,
  rec: null,
  info: null,
  D: 0, // duration
  fd: 1 / 30, // frame duration
  v0: 0, // first frame start
  edits: { inPoint: 0, outPoint: 0, cuts: [] },
  sel: null,
  cur: 0, // playhead
  undo: [],
  redo: [],
  zoom: 0,
  pps: 1,
  strip: [],
  stripCount: 0,
  wave: undefined, // undefined = loading, null = no sound
  playing: false,
  drag: null,
  exporting: false,
  lastExport: null,
  loadToken: 0,
};

// ───────────────────────────── formatting ─────────────────────────────

function fmt(t, cs = true) {
  const neg = t < 0;
  t = Math.abs(t);
  const total = cs ? Math.round(t * 100) : Math.floor(t);
  const c = cs ? total % 100 : 0;
  const secs = cs ? Math.floor(total / 100) : total;
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  const ss = String(s).padStart(2, '0');
  const tail = cs ? `.${String(c).padStart(2, '0')}` : '';
  return `${neg ? '−' : ''}${h ? `${h}:${String(m).padStart(2, '0')}` : m}:${ss}${tail}`;
}

function fmtLen(t) {
  if (t < 60) return `${t.toFixed(t < 10 ? 2 : 1)} s`;
  return fmt(t, false);
}

function formatSize(bytes) {
  if (!bytes) return '–';
  const mb = bytes / 1_000_000;
  return mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : mb >= 10 ? `${Math.round(mb)} MB` : `${mb.toFixed(1)} MB`;
}

function errorText(e) {
  return typeof e === 'string' ? e : e?.message || String(e);
}

function toast(message, kind = 'info') {
  const el = document.createElement('div');
  el.className = `toast ${kind}`;
  el.setAttribute('role', kind === 'error' ? 'alert' : 'status');
  el.innerHTML = `${icon(kind === 'error' ? 'alert' : kind === 'ok' ? 'check' : 'film')}<div class="toast-body"></div>`;
  el.querySelector('.toast-body').textContent = message;
  const close = document.createElement('button');
  close.className = 'toast-close';
  close.setAttribute('aria-label', 'Dismiss');
  close.innerHTML = icon('close');
  close.onclick = () => el.remove();
  el.append(close);
  $('#toasts').append(el);
  setTimeout(() => el.remove(), kind === 'error' ? 12000 : 5000);
}

// ───────────────────────────── frames ─────────────────────────────

const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
const lastFrame = () => Math.max(0, Math.ceil((S.D - S.v0) / S.fd - 1e-6) - 1);
const frameOf = (t) => clamp(Math.floor((t - S.v0) / S.fd + 1e-4), 0, lastFrame());
const startOf = (k) => (k <= 0 ? 0 : k > lastFrame() ? S.D : clamp(S.v0 + k * S.fd, 0, S.D));

/** The frame boundary nearest to t. */
function snap(t) {
  return startOf(Math.round((t - S.v0) / S.fd));
}

// ───────────────────────────── edit model ─────────────────────────────

function normalize(e) {
  const D = S.D;
  let inPoint = clamp(e.inPoint, 0, D);
  let outPoint = clamp(e.outPoint, 0, D);
  if (outPoint - inPoint < S.fd * 0.5) {
    inPoint = 0;
    outPoint = D;
  }
  const cuts = e.cuts
    .map((c) => ({ start: clamp(Math.min(c.start, c.end), inPoint, outPoint), end: clamp(Math.max(c.start, c.end), inPoint, outPoint) }))
    .filter((c) => c.end - c.start >= S.fd * 0.5)
    .sort((a, b) => a.start - b.start);
  const merged = [];
  for (const c of cuts) {
    const last = merged[merged.length - 1];
    if (last && c.start <= last.end + 1e-6) last.end = Math.max(last.end, c.end);
    else merged.push({ ...c });
  }
  return { inPoint, outPoint, cuts: merged };
}

function keepSpans(e = S.edits) {
  const spans = [];
  let cursor = e.inPoint;
  for (const c of e.cuts) {
    if (c.start > cursor) spans.push({ start: cursor, end: c.start });
    cursor = Math.max(cursor, c.end);
  }
  if (cursor < e.outPoint) spans.push({ start: cursor, end: e.outPoint });
  return spans.filter((s) => s.end - s.start >= 0.01);
}

const keptLength = () => keepSpans().reduce((a, s) => a + s.end - s.start, 0);

/** The removed range containing t, if any: before in, inside a cut, or from out on. */
function removedAt(t) {
  const e = S.edits;
  const eps = 1e-4;
  if (t < e.inPoint - eps) return { start: 0, end: e.inPoint };
  if (t >= e.outPoint - eps) return { start: e.outPoint, end: S.D };
  return e.cuts.find((c) => t >= c.start - eps && t < c.end - eps) || null;
}

function wireEdits() {
  return {
    inPoint: S.edits.inPoint,
    outPoint: S.edits.outPoint >= S.D - 1e-6 ? null : S.edits.outPoint,
    cuts: S.edits.cuts.map((c) => ({ start: c.start, end: c.end })),
  };
}

const snapshot = () => JSON.stringify(S.edits);

let saveTimer = 0;
function persist() {
  clearTimeout(saveTimer);
  const id = S.id;
  const edits = wireEdits();
  saveTimer = setTimeout(() => {
    invoke('editor_save_edits', { id, edits }).catch((e) => console.warn('save failed', e));
  }, 350);
}

/** Applies a change, recording `before` for undo. */
function commit(before, next) {
  S.edits = normalize(next ?? S.edits);
  if (before !== snapshot()) {
    S.undo.push(before);
    if (S.undo.length > 200) S.undo.shift();
    S.redo = [];
    persist();
  }
  render();
}

function change(mutator) {
  const before = snapshot();
  const next = JSON.parse(before);
  mutator(next);
  commit(before, next);
}

function undo() {
  if (!S.undo.length) return;
  S.redo.push(snapshot());
  S.edits = JSON.parse(S.undo.pop());
  persist();
  render();
}

function redo() {
  if (!S.redo.length) return;
  S.undo.push(snapshot());
  S.edits = JSON.parse(S.redo.pop());
  persist();
  render();
}

// ───────────────────────────── edit actions ─────────────────────────────

function setIn() {
  const t = startOf(frameOf(S.cur));
  change((e) => {
    e.inPoint = t;
    if (e.outPoint <= t) e.outPoint = S.D;
  });
}

function setOut() {
  // The frame under the playhead is the last one kept.
  const t = startOf(frameOf(S.cur) + 1);
  change((e) => {
    e.outPoint = t;
    if (e.inPoint >= t) e.inPoint = 0;
  });
}

function cutSelection() {
  if (!S.sel) return;
  const { start, end } = orderedSel();
  if (end - start < S.fd * 0.5) return;
  S.sel = null;
  change((e) => e.cuts.push({ start, end }));
  if (S.cur >= start && S.cur < end) seek(end);
  else render();
}

function restoreCut(i) {
  change((e) => e.cuts.splice(i, 1));
}

function orderedSel() {
  const a = Math.min(S.sel.start, S.sel.end);
  const b = Math.max(S.sel.start, S.sel.end);
  return { start: a, end: b };
}

// ───────────────────────────── playback ─────────────────────────────

/** Moves the playhead to the frame at t (paused) and shows that frame. */
function seek(t, { exact = false } = {}) {
  t = clamp(t, 0, S.D);
  if (!exact) t = startOf(frameOf(t));
  S.cur = t;
  if (video.readyState >= 1) {
    // Aim for the middle of the frame so the player shows exactly that frame.
    const target = t >= S.D ? S.D - S.fd * 0.5 : t + S.fd * 0.5;
    video.currentTime = clamp(target, 0, Math.max(0, (video.duration || S.D) - 0.001));
  }
  render();
}

function stepFrames(n) {
  pause();
  seek(startOf(frameOf(S.cur) + n));
  revealPlayhead();
}

function play() {
  if (!video.src || S.playing) return;
  const e = S.edits;
  let t = S.cur;
  if (t >= e.outPoint - S.fd * 0.5 || t < e.inPoint) t = e.inPoint;
  const r = removedAt(t);
  if (r) t = r.end;
  if (t >= e.outPoint) t = e.inPoint;
  S.cur = t;
  video.currentTime = t + S.fd * 0.25;
  S.playing = true;
  video.play().catch((err) => {
    S.playing = false;
    toast(`Could not play the video (${errorText(err)}).`, 'error');
    renderTransport();
  });
  renderTransport();
  requestAnimationFrame(tick);
}

function pause() {
  if (!S.playing) return;
  S.playing = false;
  video.pause();
  S.cur = startOf(frameOf(video.currentTime));
  renderTransport();
  render();
}

function togglePlay() {
  S.playing ? pause() : play();
}

let skipFlash = 0;
function tick() {
  if (!S.playing) return;
  const t = video.currentTime;
  const r = removedAt(t);
  if (r) {
    if (r.end >= S.edits.outPoint - 1e-6 || r.start >= S.edits.outPoint - 1e-6) {
      // Reached the end of what is kept.
      S.playing = false;
      video.pause();
      seek(S.edits.outPoint - S.fd * 0.5);
      renderTransport();
      return;
    }
    video.currentTime = r.end + S.fd * 0.25;
    S.cur = r.end;
    const flash = $('#stage-skip');
    flash.classList.remove('on');
    void flash.offsetWidth;
    flash.classList.add('on');
    clearTimeout(skipFlash);
    skipFlash = setTimeout(() => flash.classList.remove('on'), 500);
  } else {
    S.cur = t;
  }
  renderPlayhead();
  followPlayhead();
  requestAnimationFrame(tick);
}

video.addEventListener('ended', () => {
  if (S.playing) {
    S.playing = false;
    seek(S.edits.outPoint - S.fd * 0.5);
    renderTransport();
  }
});

video.addEventListener('pause', () => {
  // Paused by the system (e.g. the window was hidden).
  if (S.playing && !video.ended) pause();
});

video.addEventListener('error', () => {
  const code = video.error?.code;
  showStageMessage(
    code === 4
      ? 'This video format cannot be previewed here, but it can still be edited and exported.'
      : 'The video could not be loaded for preview.',
  );
});

function showStageMessage(text) {
  const m = $('#stage-msg');
  m.textContent = text;
  m.hidden = !text;
}

// ───────────────────────────── geometry ─────────────────────────────

const viewW = () => scroller.clientWidth;
const xOf = (t) => PAD + t * S.pps;
const tOf = (x) => (x - PAD) / S.pps;

function computeScale() {
  const fit = Math.max(1e-3, (viewW() - PAD * 2) / Math.max(S.D, 0.1));
  const max = Math.max(fit, (1 / S.fd) * 16); // 16 px per frame at full zoom
  S.pps = fit * Math.pow(max / fit, S.zoom);
  content.style.width = `${Math.ceil(S.D * S.pps + PAD * 2)}px`;
}

function setZoom(z, anchorX = null) {
  z = clamp(z, 0, 1);
  const ax = anchorX ?? xOf(S.cur) - scroller.scrollLeft;
  const anchorT = tOf(scroller.scrollLeft + clamp(ax, 0, viewW()));
  S.zoom = z;
  computeScale();
  scroller.scrollLeft = xOf(anchorT) - ax;
  $('#zoom').value = String(Math.round(z * 1000));
  render();
}

function revealPlayhead() {
  const x = xOf(S.cur);
  const l = scroller.scrollLeft;
  if (x < l + 24 || x > l + viewW() - 24) scroller.scrollLeft = x - viewW() / 2;
}

function followPlayhead() {
  const x = xOf(S.cur);
  const l = scroller.scrollLeft;
  if (S.zoom > 0 && (x > l + viewW() - 40 || x < l)) scroller.scrollLeft = x - 60;
}

// ───────────────────────────── drawing ─────────────────────────────

let colors = {};
function readColors() {
  const cs = getComputedStyle(document.documentElement);
  const v = (n) => cs.getPropertyValue(n).trim();
  colors = {
    ink: v('--ink'), slate: v('--slate'), mist: v('--mist'), line: v('--line'),
    lane: v('--lane'), wave: v('--wave'), paper: v('--paper'),
  };
}

let drawQueued = false;
function requestDraw() {
  if (drawQueued) return;
  drawQueued = true;
  requestAnimationFrame(() => {
    drawQueued = false;
    draw();
  });
}

function draw() {
  const dpr = window.devicePixelRatio || 1;
  const w = viewW();
  const h = canvas.clientHeight;
  if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
  }
  canvas.style.width = `${w}px`;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  if (!S.D) return;
  const sl = scroller.scrollLeft;
  drawRuler(sl, w);
  drawStrip(sl, w);
  drawWave(sl, w);
}

function roundRect(x, y, w, h, r) {
  ctx.beginPath();
  ctx.roundRect ? ctx.roundRect(x, y, w, h, r) : ctx.rect(x, y, w, h);
}

function drawRuler(sl, w) {
  const steps = [1, 2, 5, 10, 15].map((n) => n * S.fd).concat([0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600]);
  const major = steps.find((s) => s * S.pps >= 76) || 3600;
  const minorDiv = major <= 1 && major < 0.5 ? major / S.fd : 5;
  const minor = major / minorDiv;
  const t0 = Math.max(0, tOf(sl) - major);
  const t1 = Math.min(S.D, tOf(sl + w));
  ctx.font = `500 10.5px ${getComputedStyle(document.body).fontFamily}`;
  ctx.textBaseline = 'middle';
  ctx.fillStyle = colors.mist;
  if (minor * S.pps >= 5) {
    for (let t = Math.floor(t0 / minor) * minor; t <= t1 + 1e-9; t += minor) {
      ctx.fillRect(Math.round(xOf(t) - sl), RULER_H - 5, 1, 4);
    }
  }
  const showFrames = major < 0.5;
  for (let t = Math.floor(t0 / major) * major; t <= t1 + 1e-9; t += major) {
    const x = Math.round(xOf(t) - sl);
    ctx.fillStyle = colors.mist;
    ctx.fillRect(x, RULER_H - 10, 1, 9);
    ctx.fillStyle = colors.slate;
    let label;
    if (showFrames) {
      const whole = Math.floor(t + 1e-6);
      const f = Math.round((t - whole) / S.fd);
      label = f === 0 ? fmt(whole, false) : `${f}f`;
    } else {
      label = major < 1 ? fmt(t).replace(/0$/, '') : fmt(t, false);
    }
    ctx.fillText(label, x + 4, RULER_H - 15);
  }
}

function drawStrip(sl, w) {
  const x0 = xOf(0) - sl;
  const x1 = xOf(S.D) - sl;
  ctx.save();
  roundRect(x0, STRIP_TOP, x1 - x0, STRIP_H, 6);
  ctx.fillStyle = colors.lane;
  ctx.fill();
  ctx.clip();
  const aspect = S.info?.width && S.info?.height ? S.info.width / S.info.height : 16 / 9;
  const tw = clamp(STRIP_H * aspect, 40, 220);
  const first = Math.max(0, Math.floor((sl - PAD) / tw));
  for (let i = first; ; i++) {
    const x = PAD + i * tw - sl;
    if (x > w || x > x1) break;
    const tc = ((i + 0.5) * tw) / S.pps;
    const idx = S.stripCount ? Math.min(S.stripCount - 1, Math.floor((Math.min(tc, S.D) / S.D) * S.stripCount)) : -1;
    const img = idx >= 0 ? S.strip[idx] : null;
    if (img && img.complete && img.naturalWidth) {
      // Cover the slot.
      const ir = img.naturalWidth / img.naturalHeight;
      const sr = tw / STRIP_H;
      let sx = 0, sy = 0, sw = img.naturalWidth, sh = img.naturalHeight;
      if (ir > sr) { sw = sh * sr; sx = (img.naturalWidth - sw) / 2; } else { sh = sw / sr; sy = (img.naturalHeight - sh) / 2; }
      ctx.drawImage(img, sx, sy, sw, sh, x, STRIP_TOP, tw, STRIP_H);
    }
    ctx.fillStyle = 'rgba(0,0,0,0.18)';
    ctx.fillRect(Math.round(x + tw) - 1, STRIP_TOP, 1, STRIP_H);
  }
  ctx.restore();
}

function drawWave(sl, w) {
  const x0 = Math.max(xOf(0) - sl, 0);
  const x1 = Math.min(xOf(S.D) - sl, w);
  ctx.save();
  roundRect(xOf(0) - sl, WAVE_TOP, xOf(S.D) - xOf(0), WAVE_H, 6);
  ctx.fillStyle = colors.lane;
  ctx.fill();
  ctx.clip();
  const mid = WAVE_TOP + WAVE_H / 2;
  if (!S.wave) {
    ctx.fillStyle = colors.mist;
    ctx.font = `500 11.5px ${getComputedStyle(document.body).fontFamily}`;
    ctx.textBaseline = 'middle';
    const text = S.wave === null ? 'No sound in this recording' : 'Reading sound…';
    ctx.fillText(text, Math.max(x0, 0) + 12, mid);
    ctx.restore();
    return;
  }
  const { perSec, peaks } = S.wave;
  ctx.fillStyle = colors.wave;
  ctx.beginPath();
  const half = WAVE_H / 2 - 3;
  for (let px = Math.floor(x0); px < x1; px++) {
    const ta = tOf(px + sl);
    const tb = tOf(px + 1 + sl);
    const i0 = Math.max(0, Math.floor(ta * perSec));
    const i1 = Math.min(peaks.length, Math.max(i0 + 1, Math.ceil(tb * perSec)));
    let p = 0;
    for (let i = i0; i < i1; i++) if (peaks[i] > p) p = peaks[i];
    const hh = Math.max(0.5, Math.pow(p / 255, 0.65) * half);
    ctx.rect(px, mid - hh, 1, hh * 2);
  }
  ctx.fill();
  ctx.restore();
}

// ───────────────────────────── overlays & panels ─────────────────────────────

function place(el, a, b, extra = 0) {
  el.style.left = `${xOf(a)}px`;
  el.style.width = `${Math.max(extra, xOf(b) - xOf(a))}px`;
}

function renderOverlays() {
  const e = S.edits;
  const before = $('#tl-before');
  const after = $('#tl-after');
  before.hidden = e.inPoint <= 1e-6;
  after.hidden = e.outPoint >= S.D - 1e-6;
  place(before, 0, e.inPoint);
  place(after, e.outPoint, S.D);
  $('#tl-in').style.left = `${xOf(e.inPoint)}px`;
  $('#tl-out').style.left = `${xOf(e.outPoint)}px`;

  const cutsEl = $('#tl-cuts');
  cutsEl.innerHTML = '';
  e.cuts.forEach((c, i) => {
    const d = document.createElement('div');
    d.className = 'tl-cut';
    place(d, c.start, c.end, 2);
    d.innerHTML = `
      <span class="tl-handle" data-h="cut-start" data-i="${i}"></span>
      <span class="tl-handle" data-h="cut-end" data-i="${i}"></span>
      <button class="tl-cut-restore" data-restore="${i}" title="Put this part back" aria-label="Put this part back">${icon('close')}</button>`;
    cutsEl.append(d);
  });

  const sel = $('#tl-sel');
  sel.hidden = !S.sel;
  if (S.sel) {
    const { start, end } = orderedSel();
    place(sel, start, end, 2);
    $('#sel-len').textContent = fmtLen(end - start);
    sel.classList.toggle('narrow', xOf(end) - xOf(start) < 96);
  }
  $('#cut-btn').disabled = !S.sel;
  renderPlayhead();
}

function renderPlayhead() {
  $('#tl-playhead').style.left = `${xOf(S.cur)}px`;
  $('#cur-time').textContent = fmt(S.cur);
}

function renderTransport() {
  const play = $('#play');
  play.innerHTML = icon(S.playing ? 'pause' : 'play');
  play.title = S.playing ? 'Pause (Space)' : 'Play (Space)';
  play.setAttribute('aria-label', S.playing ? 'Pause' : 'Play');
  $('#undo').disabled = !S.undo.length;
  $('#redo').disabled = !S.redo.length;
}

function renderInspector() {
  const e = S.edits;
  const kept = keptLength();
  $('#result-time').textContent = fmt(kept);
  const removed = S.D - kept;
  $('#result-sub').textContent = removed > 0.005 ? `of ${fmt(S.D)} · ${fmt(removed)} removed` : `Nothing removed yet`;
  $('#in-time').textContent = fmt(e.inPoint);
  $('#out-time').textContent = fmt(e.outPoint);
  $('#in-reset').disabled = e.inPoint <= 1e-6;
  $('#out-reset').disabled = e.outPoint >= S.D - 1e-6;

  // A minimap of what is kept.
  const bar = $('#result-bar');
  bar.innerHTML = keepSpans()
    .map((s) => `<span style="left:${(s.start / S.D) * 100}%;width:${Math.max(0.3, ((s.end - s.start) / S.D) * 100)}%"></span>`)
    .join('');

  const list = $('#cut-list');
  list.innerHTML = '';
  $('#cut-count').textContent = e.cuts.length || '';
  $('#cut-hint').hidden = e.cuts.length > 0;
  e.cuts.forEach((c, i) => {
    const li = document.createElement('li');
    li.className = 'cut-item';
    li.innerHTML = `
      <button class="cut-go" title="Go to this cut">
        <span class="cut-range tabular">${fmt(c.start)} – ${fmt(c.end)}</span>
        <span class="cut-len tabular">${fmtLen(c.end - c.start)}</span>
      </button>
      <button class="icon-btn" title="Put this part back" aria-label="Put this part back">${icon('restore')}</button>`;
    li.querySelector('.cut-go').onclick = () => { pause(); seek(Math.max(0, c.start - 1)); revealPlayhead(); };
    li.querySelector('.icon-btn').onclick = () => restoreCut(i);
    list.append(li);
  });
  $('#btn-export').disabled = !S.D || kept < 0.05 || S.exporting;
}

function render() {
  if (!S.D) return;
  renderOverlays();
  renderInspector();
  renderTransport();
  requestDraw();
}

// ───────────────────────────── timeline interaction ─────────────────────────────

function pointerTime(ev) {
  const r = content.getBoundingClientRect();
  return clamp(tOf(ev.clientX - r.left), 0, S.D);
}

/** Snaps to frames, and to nearby edit points and the playhead. */
function magnet(t, ignore = []) {
  const e = S.edits;
  const targets = [0, S.D, e.inPoint, e.outPoint, S.cur, ...e.cuts.flatMap((c) => [c.start, c.end])];
  let best = null;
  for (const p of targets) {
    if (ignore.some((x) => Math.abs(x - p) < 1e-9)) continue;
    const d = Math.abs(xOf(p) - xOf(t));
    if (d <= SNAP_PX && (best === null || d < Math.abs(xOf(best) - xOf(t)))) best = p;
  }
  return best ?? snap(t);
}

function autoScroll(ev) {
  const r = scroller.getBoundingClientRect();
  const edge = 36;
  if (ev.clientX < r.left + edge) scroller.scrollLeft -= (r.left + edge - ev.clientX) * 0.6;
  else if (ev.clientX > r.right - edge) scroller.scrollLeft += (ev.clientX - (r.right - edge)) * 0.6;
}

function onPointerDown(ev) {
  if (!S.D || ev.button !== 0) return;
  const target = ev.target;
  if (target.closest('[data-restore]')) {
    restoreCut(Number(target.closest('[data-restore]').dataset.restore));
    return;
  }
  if (target.closest('#tl-sel-cut')) {
    cutSelection();
    return;
  }
  const r = content.getBoundingClientRect();
  const y = ev.clientY - r.top;
  const handle = target.closest('[data-h]');
  const before = snapshot();
  if (handle) {
    S.drag = { kind: handle.dataset.h, i: Number(handle.dataset.i ?? -1), before };
  } else if (y < RULER_H) {
    pause();
    S.drag = { kind: 'scrub' };
    seek(pointerTime(ev));
  } else {
    S.drag = { kind: 'press', x: ev.clientX, t: pointerTime(ev) };
  }
  document.body.classList.add('dragging');
  scroller.setPointerCapture(ev.pointerId);
  ev.preventDefault();
}

function onPointerMove(ev) {
  const t = S.D ? pointerTime(ev) : 0;
  // Hover time bubble.
  const hover = $('#tl-hover');
  if (S.D && (!S.drag || S.drag.kind === 'scrub')) {
    hover.hidden = false;
    hover.style.left = `${xOf(t)}px`;
    hover.querySelector('span').textContent = fmt(snap(t));
  }
  const d = S.drag;
  if (!d) return;
  autoScroll(ev);
  const e = S.edits;
  switch (d.kind) {
    case 'scrub':
      seek(t);
      return;
    case 'press':
      if (Math.abs(ev.clientX - d.x) < 4) return;
      pause();
      d.kind = 'sel-end';
      S.sel = { start: magnet(d.t), end: magnet(t) };
      break;
    case 'sel-start':
    case 'sel-end': {
      const key = d.kind === 'sel-start' ? 'start' : 'end';
      const other = S.sel[key === 'start' ? 'end' : 'start'];
      S.sel[key] = magnet(t, [other]);
      break;
    }
    case 'in':
      e.inPoint = clamp(magnet(t, [e.inPoint]), 0, e.outPoint - S.fd);
      break;
    case 'out':
      e.outPoint = clamp(magnet(t, [e.outPoint]), e.inPoint + S.fd, S.D);
      break;
    case 'cut-start': {
      const c = e.cuts[d.i];
      if (c) c.start = clamp(magnet(t, [c.start]), e.inPoint, c.end - S.fd);
      break;
    }
    case 'cut-end': {
      const c = e.cuts[d.i];
      if (c) c.end = clamp(magnet(t, [c.end]), c.start + S.fd, e.outPoint);
      break;
    }
  }
  renderOverlays();
  renderInspector();
}

function onPointerUp(ev) {
  const d = S.drag;
  S.drag = null;
  document.body.classList.remove('dragging');
  if (!d) return;
  try { scroller.releasePointerCapture(ev.pointerId); } catch { /* already released */ }
  if (d.kind === 'press') {
    // A click: move the playhead there.
    pause();
    S.sel = null;
    seek(pointerTime(ev));
    return;
  }
  if (d.kind === 'sel-start' || d.kind === 'sel-end') {
    if (S.sel && Math.abs(S.sel.end - S.sel.start) < S.fd * 0.5) S.sel = null;
    render();
    return;
  }
  if (d.before) commit(d.before);
}

scroller.addEventListener('pointerdown', onPointerDown);
scroller.addEventListener('pointermove', onPointerMove);
scroller.addEventListener('pointerup', onPointerUp);
scroller.addEventListener('pointercancel', onPointerUp);
scroller.addEventListener('pointerleave', () => { if (!S.drag) $('#tl-hover').hidden = true; });
scroller.addEventListener('scroll', requestDraw);
scroller.addEventListener(
  'wheel',
  (ev) => {
    if (!S.D) return;
    if (ev.ctrlKey || ev.metaKey) {
      ev.preventDefault();
      const r = scroller.getBoundingClientRect();
      setZoom(S.zoom - ev.deltaY * 0.0015, ev.clientX - r.left);
    } else if (Math.abs(ev.deltaY) > Math.abs(ev.deltaX) && S.zoom > 0) {
      ev.preventDefault();
      scroller.scrollLeft += ev.deltaY;
    }
  },
  { passive: false },
);

// ───────────────────────────── export ─────────────────────────────

function showPane(name) {
  document.querySelectorAll('.ex-pane').forEach((p) => { p.hidden = p.dataset.pane !== name; });
}

function openExport() {
  if ($('#btn-export').disabled) return;
  pause();
  const kept = keptLength();
  $('#ex-length').textContent = fmt(kept);
  $('#ex-removed').textContent = fmt(S.D - kept);
  const est = S.rec?.sizeBytes ? (S.rec.sizeBytes * kept) / S.D : 0;
  $('#ex-size').textContent = est ? `≈ ${formatSize(est)}` : '–';
  $('#export-heading').textContent = 'Export video';
  showPane('setup');
  $('#export-sheet').showModal();
  $('#ex-start').focus();
}

async function startExport() {
  S.exporting = true;
  renderInspector();
  $('#export-heading').textContent = 'Exporting';
  $('#ex-stage').textContent = 'Preparing…';
  $('#ex-bar').style.width = '0%';
  $('#ex-pct').textContent = '0%';
  showPane('running');
  const started = performance.now();
  try {
    const result = await invoke('editor_export', { id: S.id, edits: wireEdits(), reencode: $('#ex-reencode').checked });
    S.lastExport = result.recording;
    const r = result.report;
    const secs = Math.max(1, Math.round((performance.now() - started) / 1000));
    $('#export-heading').textContent = 'Export finished';
    $('#ex-done-title').textContent = result.recording.title;
    const copied = r.duration > 0 ? Math.round((1 - r.reencoded / r.duration) * 100) : 0;
    const how =
      r.method === 'copy'
        ? 'Every frame was copied as it was: no quality lost.'
        : r.method === 'smart'
          ? `${copied}% was copied as it was; only the frames around the cuts were re-encoded.`
          : 'Every frame was re-encoded.';
    $('#ex-done-sub').textContent = `${fmt(r.duration)} long, done in ${secs} s. ${how}`;
    showPane('done');
    $('#ex-done').focus();
  } catch (e) {
    const msg = errorText(e);
    $('#export-sheet').close();
    if (msg !== 'cancelled') toast(`The export failed.\n\n${msg}`, 'error');
    else toast('Export stopped.');
  } finally {
    S.exporting = false;
    renderInspector();
  }
}

function wireExport() {
  $('#btn-export').onclick = openExport;
  $('#ex-start').onclick = startExport;
  $('#ex-close').onclick = () => $('#export-sheet').close();
  $('#ex-done').onclick = () => $('#export-sheet').close();
  $('#ex-cancel').onclick = () => {
    $('#ex-stage').textContent = 'Stopping…';
    invoke('editor_cancel_export');
  };
  $('#ex-reveal').onclick = () => S.lastExport && invoke('reveal_recording', { id: S.lastExport.id }).catch((e) => toast(errorText(e), 'error'));
  $('#ex-play').onclick = () => S.lastExport && invoke('open_recording', { id: S.lastExport.id }).catch((e) => toast(errorText(e), 'error'));
  // Esc and the close button must not hide a running export.
  $('#export-sheet').addEventListener('cancel', (ev) => { if (S.exporting) ev.preventDefault(); });
  $('#export-x').addEventListener('click', (ev) => { if (S.exporting) ev.preventDefault(); });
  try { $('#ex-reencode').checked = localStorage.getItem('tallycast.reencode') === '1'; } catch { /* storage unavailable */ }
  $('#ex-reencode').onchange = (e) => { try { localStorage.setItem('tallycast.reencode', e.target.checked ? '1' : '0'); } catch { /* ignore */ } };
}

// ───────────────────────────── wiring ─────────────────────────────

function wireControls() {
  const win = currentWindow();
  $('#btn-min').onclick = () => win?.minimize();
  $('#btn-max').onclick = () => win?.toggleMaximize();
  $('#btn-close').onclick = async () => {
    if (S.exporting) await invoke('editor_cancel_export').catch(() => {});
    clearTimeout(saveTimer);
    if (S.id) await invoke('editor_save_edits', { id: S.id, edits: wireEdits() }).catch(() => {});
    win?.close();
  };
  $('#play').onclick = togglePlay;
  $('#step-back').onclick = () => stepFrames(-1);
  $('#step-fwd').onclick = () => stepFrames(1);
  $('#go-in').onclick = () => { pause(); seek(S.edits.inPoint); revealPlayhead(); };
  $('#go-out').onclick = () => { pause(); seek(S.edits.outPoint - S.fd * 0.5); revealPlayhead(); };
  $('#in-set').onclick = setIn;
  $('#out-set').onclick = setOut;
  $('#in-reset').onclick = () => change((e) => { e.inPoint = 0; });
  $('#out-reset').onclick = () => change((e) => { e.outPoint = S.D; });
  $('#cut-btn').onclick = cutSelection;
  $('#undo').onclick = undo;
  $('#redo').onclick = redo;
  $('#zoom').oninput = (e) => setZoom(Number(e.target.value) / 1000);
  $('#zoom-in').onclick = () => setZoom(S.zoom + 0.1);
  $('#zoom-out').onclick = () => setZoom(S.zoom - 0.1);
  $('#viewport').onclick = (ev) => { if (ev.target === video) togglePlay(); };

  window.addEventListener('keydown', (ev) => {
    if (!S.D || $('#export-sheet').open) return;
    if (ev.target.closest?.('input, select, textarea')) return;
    const k = ev.key;
    const ctrl = ev.ctrlKey || ev.metaKey;
    let handled = true;
    if (k === ' ' || k === 'k' || k === 'K') togglePlay();
    else if (k === 'ArrowLeft') ev.shiftKey ? (pause(), seek(S.cur - 1), revealPlayhead()) : stepFrames(-1);
    else if (k === 'ArrowRight') ev.shiftKey ? (pause(), seek(S.cur + 1), revealPlayhead()) : stepFrames(1);
    else if (k === 'Home') $('#go-in').click();
    else if (k === 'End') $('#go-out').click();
    else if ((k === 'i' || k === 'I') && !ctrl) setIn();
    else if ((k === 'o' || k === 'O') && !ctrl) setOut();
    else if (k === 'Delete' || k === 'Backspace' || ((k === 'x' || k === 'X') && !ctrl)) cutSelection();
    else if (k === 'Escape') { S.sel = null; render(); }
    else if (ctrl && (k === 'z' || k === 'Z')) ev.shiftKey ? redo() : undo();
    else if (ctrl && (k === 'y' || k === 'Y')) redo();
    else if (ctrl && (k === 'e' || k === 'E')) openExport();
    else if (k === '+' || k === '=') setZoom(S.zoom + 0.1);
    else if (k === '-' || k === '_') setZoom(S.zoom - 0.1);
    else handled = false;
    if (handled) ev.preventDefault();
  });

  new ResizeObserver(() => {
    if (!S.D) return;
    const t = tOf(scroller.scrollLeft);
    computeScale();
    scroller.scrollLeft = xOf(t) - PAD;
    render();
  }).observe(scroller);
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => { readColors(); requestDraw(); });
}

// ───────────────────────────── loading ─────────────────────────────

function stripCountFor(duration) {
  return clamp(Math.ceil(duration / 1.5), 16, 160);
}

async function load(id) {
  const token = ++S.loadToken;
  pause();
  S.id = id;
  S.D = 0;
  S.sel = null;
  S.undo = [];
  S.redo = [];
  S.strip = [];
  S.wave = undefined;
  S.cur = 0;
  showStageMessage('');
  document.body.classList.add('loading');
  let data;
  try {
    data = await invoke('editor_load', { id });
  } catch (e) {
    document.body.classList.remove('loading');
    showStageMessage(errorText(e));
    return;
  }
  if (token !== S.loadToken) return;
  S.rec = data.recording;
  S.info = data.info;
  S.D = data.info.duration;
  S.fd = 1 / (data.info.fps || 30);
  S.v0 = data.info.videoStart || 0;
  const saved = data.edits || {};
  S.edits = normalize({
    inPoint: saved.inPoint || 0,
    outPoint: saved.outPoint ?? S.D,
    cuts: saved.cuts || [],
  });
  $('#ed-title').textContent = S.rec.title;
  document.title = `${S.rec.title} – Tallycast Editor`;
  $('#dur-time').textContent = ` / ${fmt(S.D)}`;
  video.src = data.mediaUrl || '';
  S.zoom = 0;
  $('#zoom').value = '0';
  computeScale();
  scroller.scrollLeft = 0;
  document.body.classList.remove('loading');
  render();

  // Timeline media arrive in the background.
  S.stripCount = stripCountFor(S.D);
  const height = Math.round(STRIP_H * Math.min(2, window.devicePixelRatio || 1));
  invoke('editor_filmstrip', { id, count: S.stripCount, height }).catch((e) => console.warn('filmstrip', e));
  invoke('editor_waveform', { id })
    .then((w) => {
      if (token !== S.loadToken) return;
      S.wave = w ? { perSec: w.perSec, peaks: Uint8Array.from(w.peaks) } : null;
      requestDraw();
    })
    .catch(() => { S.wave = null; requestDraw(); });
}

video.addEventListener('loadedmetadata', () => seek(S.cur));

async function init() {
  hydrateIcons();
  readColors();
  wireControls();
  wireExport();
  await listen('filmstrip-frame', (f) => {
    if (f.id !== S.id) return;
    const img = new Image();
    img.onload = requestDraw;
    img.src = f.src;
    S.strip[f.index] = img;
  });
  await listen('export-progress', (p) => {
    const pct = Math.round(p.fraction * 100);
    $('#ex-bar').style.width = `${pct}%`;
    $('#ex-pct').textContent = `${pct}%`;
    $('#ex-progress').setAttribute('aria-valuenow', String(pct));
    if ($('#ex-stage').textContent !== 'Stopping…') {
      $('#ex-stage').textContent = p.stage === 'preparing' ? 'Reading the video…' : p.stage === 'finishing' ? 'Finishing…' : 'Exporting…';
    }
  });
  await listen('editor-load', (id) => {
    if (S.exporting) {
      toast('Finish or stop the export before opening another recording.', 'error');
      return;
    }
    if ($('#export-sheet').open) $('#export-sheet').close();
    load(String(id));
  });
  const id = new URLSearchParams(location.search).get('id');
  if (id) await load(id);
  else showStageMessage('Open a recording from the library to edit it.');
}

init().catch((e) => toast(`The editor could not start: ${errorText(e)}`, 'error'));
