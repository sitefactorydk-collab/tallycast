// Tallycast screenshot annotation.
//
// Marks are kept as shapes in image pixels on top of the untouched capture, so they
// stay editable; Save renders them into the PNG (blur replaces the pixels for real).

import { invoke, listen, currentWindow } from './tauri.js';
import { icon, hydrateIcons } from './icons.js';

const $ = (sel) => document.querySelector(sel);
const stage = $('#stage');
const canvas = $('#canvas');
const ctx = canvas.getContext('2d');
const textInput = $('#text-input');

const COLORS = ['#e23b3e', '#f5b800', '#2a57de', '#1f9d55', '#18212e', '#ffffff'];
const COLOR_NAMES = ['Red', 'Yellow', 'Blue', 'Green', 'Black', 'White'];
const HINTS = {
  select: 'Click a mark to select it, drag to move it, double-click text to edit it.',
  arrow: 'Drag from the tail to the point of the arrow.',
  box: 'Drag to draw a box around something.',
  highlight: 'Drag over something to highlight it.',
  text: 'Click where the text should go, type, then press Enter.',
  blur: 'Drag over anything private to blur it.',
};

const S = {
  id: null,
  rec: null,
  img: null,
  W: 0,
  H: 0,
  shapes: [],
  undo: [],
  redo: [],
  tool: 'arrow',
  // Each tool remembers its own colour and size.
  style: {
    arrow: { color: COLORS[0], size: 'm' },
    box: { color: COLORS[0], size: 'm' },
    highlight: { color: COLORS[1], size: 'm' },
    text: { color: COLORS[0], size: 'm' },
    blur: { color: null, size: 'm' },
  },
  selected: -1,
  drag: null,
  editing: null, // { index | -1, x, y } while the text box is open
  dirty: false,
  saving: false,
  view: { scale: 1, ox: 0, oy: 0 },
  loadToken: 0,
};

// ───────────────────────────── helpers ─────────────────────────────

function errorText(e) {
  return typeof e === 'string' ? e : e?.message || String(e);
}

function toast(message, kind = 'info') {
  const el = document.createElement('div');
  el.className = `toast ${kind}`;
  el.setAttribute('role', kind === 'error' ? 'alert' : 'status');
  el.innerHTML = `${icon(kind === 'error' ? 'alert' : 'check')}<div class="toast-body"></div>`;
  el.querySelector('.toast-body').textContent = message;
  $('#toasts').append(el);
  setTimeout(() => el.remove(), kind === 'error' ? 12000 : 3000);
}

/** Size units relative to the image, so marks look the same on any capture size. */
const unit = () => Math.max(S.W, S.H) / 1000;
const strokeOf = (size) => Math.max(2, ({ s: 2.5, m: 4, l: 7 })[size] * unit());
const fontOf = (size) => Math.max(12, ({ s: 16, m: 24, l: 36 })[size] * unit());
const blockOf = (size) => Math.max(6, ({ s: 8, m: 14, l: 24 })[size] * unit());

function isLight(hex) {
  const n = parseInt(hex.slice(1), 16);
  return (0.299 * (n >> 16) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255)) > 160;
}

function fontCss(size) {
  return `700 ${fontOf(size)}px "Segoe UI Variable Display", "Segoe UI", system-ui, sans-serif`;
}

// ───────────────────────────── drawing ─────────────────────────────

function pixelate(c, s) {
  const x = Math.max(0, Math.floor(Math.min(s.x, s.x + s.w)));
  const y = Math.max(0, Math.floor(Math.min(s.y, s.y + s.h)));
  const w = Math.min(S.W - x, Math.ceil(Math.abs(s.w)));
  const h = Math.min(S.H - y, Math.ceil(Math.abs(s.h)));
  if (w < 1 || h < 1) return;
  const block = blockOf(s.size);
  const sw = Math.max(1, Math.round(w / block));
  const sh = Math.max(1, Math.round(h / block));
  const small = document.createElement('canvas');
  small.width = sw;
  small.height = sh;
  const g = small.getContext('2d');
  g.imageSmoothingEnabled = true;
  g.imageSmoothingQuality = 'high';
  g.drawImage(S.img, x, y, w, h, 0, 0, sw, sh);
  c.save();
  c.imageSmoothingEnabled = false;
  c.drawImage(small, 0, 0, sw, sh, x, y, w, h);
  c.restore();
}

function drawArrow(c, s) {
  const lw = strokeOf(s.size);
  const dx = s.x2 - s.x1;
  const dy = s.y2 - s.y1;
  const len = Math.hypot(dx, dy);
  if (len < 1) return;
  const ux = dx / len;
  const uy = dy / len;
  const head = Math.min(len * 0.6, lw * 4.5);
  const wing = head * 0.62;
  const bx = s.x2 - ux * head;
  const by = s.y2 - uy * head;
  c.save();
  c.strokeStyle = c.fillStyle = s.color;
  c.lineWidth = lw;
  c.lineCap = 'round';
  c.lineJoin = 'round';
  c.shadowColor = 'rgba(0,0,0,0.28)';
  c.shadowBlur = lw * 1.2;
  c.beginPath();
  c.moveTo(s.x1, s.y1);
  c.lineTo(bx + ux * lw * 0.5, by + uy * lw * 0.5);
  c.stroke();
  c.beginPath();
  c.moveTo(s.x2, s.y2);
  c.lineTo(bx - uy * wing, by + ux * wing);
  c.lineTo(bx + uy * wing, by - ux * wing);
  c.closePath();
  c.fill();
  c.restore();
}

function drawBox(c, s) {
  const lw = strokeOf(s.size);
  c.save();
  c.strokeStyle = s.color;
  c.lineWidth = lw;
  c.lineJoin = 'round';
  c.shadowColor = 'rgba(0,0,0,0.25)';
  c.shadowBlur = lw;
  c.beginPath();
  c.roundRect ? c.roundRect(s.x, s.y, s.w, s.h, lw * 1.2) : c.rect(s.x, s.y, s.w, s.h);
  c.stroke();
  c.restore();
}

function drawHighlight(c, s) {
  c.save();
  c.globalCompositeOperation = 'multiply';
  c.globalAlpha = 0.42;
  c.fillStyle = s.color;
  c.fillRect(s.x, s.y, s.w, s.h);
  c.restore();
}

function textLines(s) {
  return String(s.text).split('\n');
}

function textBox(c, s) {
  const fs = fontOf(s.size);
  c.save();
  c.font = fontCss(s.size);
  const lines = textLines(s);
  const w = Math.max(...lines.map((l) => c.measureText(l).width), fs * 0.5);
  c.restore();
  return { x: s.x, y: s.y, w, h: lines.length * fs * 1.25 };
}

function drawText(c, s) {
  const fs = fontOf(s.size);
  c.save();
  c.font = fontCss(s.size);
  c.textBaseline = 'top';
  c.lineJoin = 'round';
  c.lineWidth = fs * 0.16;
  // A halo in the opposite tone keeps text readable on any background.
  c.strokeStyle = isLight(s.color) ? 'rgba(16,20,28,0.85)' : 'rgba(255,255,255,0.92)';
  c.fillStyle = s.color;
  textLines(s).forEach((line, i) => {
    const y = s.y + i * fs * 1.25;
    c.strokeText(line, s.x, y);
    c.fillText(line, s.x, y);
  });
  c.restore();
}

function drawShape(c, s) {
  switch (s.type) {
    case 'blur': return pixelate(c, s);
    case 'arrow': return drawArrow(c, s);
    case 'box': return drawBox(c, s);
    case 'highlight': return drawHighlight(c, s);
    case 'text': return drawText(c, s);
  }
}

/** Draws the image and every mark on `c`, in image pixels. */
function paint(c, skip = -1) {
  c.drawImage(S.img, 0, 0, S.W, S.H);
  S.shapes.forEach((s, i) => { if (i !== skip) drawShape(c, s); });
}

function normRect(s) {
  return { x: Math.min(s.x, s.x + s.w), y: Math.min(s.y, s.y + s.h), w: Math.abs(s.w), h: Math.abs(s.h) };
}

function bounds(s) {
  switch (s.type) {
    case 'arrow': {
      const pad = strokeOf(s.size) * 2.5;
      const x = Math.min(s.x1, s.x2) - pad;
      const y = Math.min(s.y1, s.y2) - pad;
      return { x, y, w: Math.abs(s.x2 - s.x1) + pad * 2, h: Math.abs(s.y2 - s.y1) + pad * 2 };
    }
    case 'text': return textBox(ctx, s);
    default: return normRect(s);
  }
}

function layout() {
  const r = stage.getBoundingClientRect();
  const pad = 24;
  const scale = S.W ? Math.min(1, (r.width - pad * 2) / S.W, (r.height - pad * 2) / S.H) : 1;
  S.view = { scale, ox: (r.width - S.W * scale) / 2, oy: (r.height - S.H * scale) / 2 };
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.round(r.width * dpr);
  canvas.height = Math.round(r.height * dpr);
  canvas.style.width = `${r.width}px`;
  canvas.style.height = `${r.height}px`;
  draw();
}

let queued = false;
function draw() {
  if (queued) return;
  queued = true;
  requestAnimationFrame(() => {
    queued = false;
    render();
  });
}

function render() {
  const dpr = window.devicePixelRatio || 1;
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  if (!S.img) return;
  const { scale, ox, oy } = S.view;
  ctx.setTransform(dpr * scale, 0, 0, dpr * scale, dpr * ox, dpr * oy);
  ctx.save();
  ctx.shadowColor = 'rgba(0,0,0,0.35)';
  ctx.shadowBlur = 18 / scale;
  ctx.fillStyle = '#000';
  ctx.fillRect(0, 0, S.W, S.H);
  ctx.restore();
  ctx.imageSmoothingQuality = 'high';
  paint(ctx, S.editing ? S.editing.index : -1);
  if (S.drag?.preview) drawShape(ctx, S.drag.preview);
  if (S.selected >= 0 && S.shapes[S.selected] && !S.editing) {
    const b = bounds(S.shapes[S.selected]);
    const pad = 6 / scale;
    ctx.save();
    ctx.lineWidth = 1.5 / scale;
    ctx.setLineDash([5 / scale, 4 / scale]);
    ctx.strokeStyle = '#ffffff';
    ctx.strokeRect(b.x - pad, b.y - pad, b.w + pad * 2, b.h + pad * 2);
    ctx.lineDashOffset = 4.5 / scale;
    ctx.strokeStyle = '#2a57de';
    ctx.strokeRect(b.x - pad, b.y - pad, b.w + pad * 2, b.h + pad * 2);
    ctx.restore();
  }
}

// ───────────────────────────── editing model ─────────────────────────────

const snapshot = () => JSON.stringify(S.shapes);

function commit(before) {
  if (before === snapshot()) return;
  S.undo.push(before);
  if (S.undo.length > 200) S.undo.shift();
  S.redo = [];
  setDirty(true);
  renderChrome();
  draw();
}

function undo() {
  if (!S.undo.length) return;
  closeText(false);
  S.redo.push(snapshot());
  S.shapes = JSON.parse(S.undo.pop());
  S.selected = -1;
  setDirty(true);
  renderChrome();
  draw();
}

function redo() {
  if (!S.redo.length) return;
  closeText(false);
  S.undo.push(snapshot());
  S.shapes = JSON.parse(S.redo.pop());
  S.selected = -1;
  setDirty(true);
  renderChrome();
  draw();
}

function deleteSelected() {
  if (S.selected < 0) return;
  const before = snapshot();
  S.shapes.splice(S.selected, 1);
  S.selected = -1;
  commit(before);
}

function setDirty(d) {
  S.dirty = d;
  $('#status-state').textContent = S.saving ? 'Saving…' : d ? 'Unsaved changes' : 'Saved';
  $('#status-state').classList.toggle('pending', d || S.saving);
}

// ───────────────────────────── hit testing ─────────────────────────────

function toImage(ev) {
  const r = stage.getBoundingClientRect();
  const { scale, ox, oy } = S.view;
  return { x: (ev.clientX - r.left - ox) / scale, y: (ev.clientY - r.top - oy) / scale };
}

function distToSegment(p, a, b) {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const t = Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy || 1)));
  return Math.hypot(p.x - (a.x + t * dx), p.y - (a.y + t * dy));
}

function hit(p) {
  const slack = 8 / S.view.scale;
  for (let i = S.shapes.length - 1; i >= 0; i--) {
    const s = S.shapes[i];
    if (s.type === 'arrow') {
      if (distToSegment(p, { x: s.x1, y: s.y1 }, { x: s.x2, y: s.y2 }) <= strokeOf(s.size) + slack) return i;
      continue;
    }
    const b = bounds(s);
    if (s.type === 'box') {
      // A box is picked by its outline, so marks inside it stay reachable.
      const inside = p.x >= b.x - slack && p.x <= b.x + b.w + slack && p.y >= b.y - slack && p.y <= b.y + b.h + slack;
      const deep = p.x > b.x + slack && p.x < b.x + b.w - slack && p.y > b.y + slack && p.y < b.y + b.h - slack;
      if (inside && !deep) return i;
      continue;
    }
    if (p.x >= b.x - slack && p.x <= b.x + b.w + slack && p.y >= b.y - slack && p.y <= b.y + b.h + slack) return i;
  }
  return -1;
}

function move(s, dx, dy) {
  if (s.type === 'arrow') {
    s.x1 += dx; s.y1 += dy; s.x2 += dx; s.y2 += dy;
  } else {
    s.x += dx; s.y += dy;
  }
}

// ───────────────────────────── text box ─────────────────────────────

function openText(index, x, y) {
  const st = index >= 0 ? S.shapes[index] : { text: '', color: S.style.text.color, size: S.style.text.size };
  S.editing = { index, x, y, color: st.color, size: st.size };
  S.selected = -1;
  const { scale, ox, oy } = S.view;
  const fs = fontOf(st.size) * scale;
  Object.assign(textInput.style, {
    left: `${ox + x * scale}px`,
    top: `${oy + y * scale}px`,
    fontSize: `${fs}px`,
    lineHeight: '1.25',
    color: st.color,
    WebkitTextStroke: '0',
    textShadow: isLight(st.color) ? '0 0 3px rgba(16,20,28,.9)' : '0 0 3px rgba(255,255,255,.95)',
  });
  textInput.value = st.text;
  textInput.hidden = false;
  autosize();
  draw();
  // Focus right away so the first key typed is not lost.
  textInput.focus();
  textInput.select();
}

function autosize() {
  textInput.style.height = 'auto';
  textInput.style.width = 'auto';
  textInput.style.height = `${textInput.scrollHeight}px`;
  textInput.style.width = `${Math.max(60, textInput.scrollWidth + 8)}px`;
}

function closeText(keep = true) {
  const e = S.editing;
  if (!e) return;
  S.editing = null;
  textInput.hidden = true;
  const text = textInput.value.replace(/\s+$/, '');
  const before = snapshot();
  if (e.index >= 0) {
    if (!keep) { draw(); return; }
    if (text) S.shapes[e.index].text = text;
    else S.shapes.splice(e.index, 1);
  } else if (keep && text) {
    S.shapes.push({ type: 'text', x: e.x, y: e.y, text, color: e.color, size: e.size });
    S.selected = S.shapes.length - 1;
  }
  commit(before);
  draw();
}

textInput.addEventListener('input', autosize);
textInput.addEventListener('keydown', (ev) => {
  if (ev.key === 'Enter' && !ev.shiftKey) { ev.preventDefault(); closeText(true); }
  else if (ev.key === 'Escape') { ev.preventDefault(); closeText(false); }
  ev.stopPropagation();
});
textInput.addEventListener('blur', () => closeText(true));

// ───────────────────────────── pointer ─────────────────────────────

stage.addEventListener('pointerdown', (ev) => {
  if (!S.img || ev.button !== 0 || ev.target === textInput) return;
  if (S.editing) { closeText(true); return; }
  const p = toImage(ev);
  const tool = S.tool;
  stage.setPointerCapture(ev.pointerId);
  if (tool === 'select') {
    S.selected = hit(p);
    if (S.selected >= 0) S.drag = { kind: 'move', last: p, before: snapshot() };
    renderChrome();
    draw();
    return;
  }
  if (tool === 'text') {
    ev.preventDefault(); // keep the focus in the text box that opens now
    const i = hit(p);
    if (i >= 0 && S.shapes[i].type === 'text') openText(i, S.shapes[i].x, S.shapes[i].y);
    else openText(-1, p.x, p.y);
    return;
  }
  const st = S.style[tool];
  S.selected = -1;
  const base = tool === 'arrow'
    ? { type: 'arrow', x1: p.x, y1: p.y, x2: p.x, y2: p.y, color: st.color, size: st.size }
    : { type: tool, x: p.x, y: p.y, w: 0, h: 0, color: st.color, size: st.size };
  S.drag = { kind: 'draw', start: p, preview: base };
  renderChrome();
});

stage.addEventListener('pointermove', (ev) => {
  const d = S.drag;
  if (!d) {
    if (S.tool === 'select' && S.img) stage.style.cursor = hit(toImage(ev)) >= 0 ? 'move' : 'default';
    return;
  }
  const p = toImage(ev);
  if (d.kind === 'move') {
    move(S.shapes[S.selected], p.x - d.last.x, p.y - d.last.y);
    d.last = p;
  } else {
    const s = d.preview;
    const x = Math.max(0, Math.min(S.W, p.x));
    const y = Math.max(0, Math.min(S.H, p.y));
    if (s.type === 'arrow') { s.x2 = x; s.y2 = y; }
    else { s.w = x - d.start.x; s.h = y - d.start.y; }
  }
  draw();
});

function endDrag() {
  const d = S.drag;
  S.drag = null;
  if (!d) return;
  if (d.kind === 'move') {
    commit(d.before);
    return;
  }
  const s = d.preview;
  const minLen = 6 / S.view.scale;
  const big = s.type === 'arrow' ? Math.hypot(s.x2 - s.x1, s.y2 - s.y1) >= minLen : Math.abs(s.w) >= minLen && Math.abs(s.h) >= minLen;
  if (!big) { draw(); return; }
  const before = snapshot();
  S.shapes.push(s.type === 'arrow' ? s : { ...s, ...normRect(s) });
  commit(before);
}

stage.addEventListener('pointerup', endDrag);
stage.addEventListener('pointercancel', endDrag);
stage.addEventListener('dblclick', (ev) => {
  if (S.tool !== 'select') return;
  const i = hit(toImage(ev));
  if (i >= 0 && S.shapes[i].type === 'text') openText(i, S.shapes[i].x, S.shapes[i].y);
});

// ───────────────────────────── toolbar ─────────────────────────────

function setTool(tool) {
  closeText(true);
  S.tool = tool;
  if (tool !== 'select') S.selected = -1;
  stage.dataset.tool = tool;
  stage.style.cursor = '';
  renderChrome();
  draw();
}

/** Colour/size changes apply to the selected mark too. */
function applyStyle(patch) {
  const sel = S.selected >= 0 ? S.shapes[S.selected] : null;
  const tool = sel ? sel.type : S.tool;
  if (S.style[tool]) Object.assign(S.style[tool], patch);
  if (sel) {
    const before = snapshot();
    if (patch.color && sel.type !== 'blur') sel.color = patch.color;
    if (patch.size) sel.size = patch.size;
    commit(before);
  }
  renderChrome();
}

function renderChrome() {
  document.querySelectorAll('[data-tool]').forEach((b) => b.setAttribute('aria-checked', String(b.dataset.tool === S.tool)));
  const sel = S.selected >= 0 ? S.shapes[S.selected] : null;
  const styleTool = sel ? sel.type : S.tool;
  const st = S.style[styleTool];
  const sw = $('#swatches');
  sw.classList.toggle('off', !st || st.color === null);
  sw.querySelectorAll('button').forEach((b) => b.setAttribute('aria-checked', String(!!st && b.dataset.color === st.color)));
  $('#sizes').classList.toggle('off', !st);
  document.querySelectorAll('[data-size]').forEach((b) => b.setAttribute('aria-checked', String(!!st && b.dataset.size === st.size)));
  $('#undo').disabled = !S.undo.length;
  $('#redo').disabled = !S.redo.length;
  $('#delete').disabled = !sel;
  $('#status-hint').textContent = HINTS[S.tool];
}

function buildSwatches() {
  const sw = $('#swatches');
  COLORS.forEach((c, i) => {
    const b = document.createElement('button');
    b.setAttribute('role', 'radio');
    b.dataset.color = c;
    b.title = COLOR_NAMES[i];
    b.setAttribute('aria-label', COLOR_NAMES[i]);
    b.style.setProperty('--c', c);
    b.onclick = () => applyStyle({ color: c });
    sw.append(b);
  });
}

// ───────────────────────────── save / copy ─────────────────────────────

function exportBlob() {
  const c = document.createElement('canvas');
  c.width = S.W;
  c.height = S.H;
  const g = c.getContext('2d');
  g.imageSmoothingQuality = 'high';
  paint(g);
  return new Promise((resolve, reject) => c.toBlob((b) => (b ? resolve(b) : reject(new Error('The image could not be rendered.'))), 'image/png'));
}

async function blobToBase64(blob) {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let bin = '';
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  return btoa(bin);
}

async function save({ quiet = false } = {}) {
  if (!S.img || S.saving) return false;
  closeText(true);
  S.saving = true;
  setDirty(S.dirty);
  try {
    const png = await blobToBase64(await exportBlob());
    await invoke('shot_save', { id: S.id, png, annotations: { version: 1, shapes: S.shapes } });
    S.saving = false;
    setDirty(false);
    if (!quiet) toast('Saved.', 'ok');
    return true;
  } catch (e) {
    S.saving = false;
    setDirty(true);
    toast(`Could not save: ${errorText(e)}`, 'error');
    return false;
  }
}

async function copy() {
  if (!S.img) return;
  closeText(true);
  try {
    const blob = await exportBlob();
    await navigator.clipboard.write([new ClipboardItem({ 'image/png': blob })]);
    toast('Copied. Paste it anywhere with Ctrl+V.', 'ok');
  } catch (e) {
    toast(`Could not copy: ${errorText(e)}`, 'error');
  }
}

// ───────────────────────────── loading ─────────────────────────────

function loadImage(url) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.crossOrigin = 'anonymous'; // keeps the canvas exportable
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error('The screenshot could not be loaded.'));
    img.src = url;
  });
}

async function load(id) {
  const token = ++S.loadToken;
  if (S.dirty && S.id && S.id !== id) await save({ quiet: true });
  S.id = id;
  S.img = null;
  S.shapes = [];
  S.undo = [];
  S.redo = [];
  S.selected = -1;
  $('#stage-msg').hidden = true;
  document.body.classList.add('loading');
  try {
    const data = await invoke('shot_load', { id });
    const img = await loadImage(data.imageUrl);
    if (token !== S.loadToken) return;
    S.rec = data.recording;
    S.img = img;
    S.W = img.naturalWidth;
    S.H = img.naturalHeight;
    S.shapes = Array.isArray(data.annotations?.shapes) ? data.annotations.shapes : [];
    $('#title').textContent = S.rec.title;
    document.title = `${S.rec.title} – Tallycast Annotate`;
    $('#status-size').textContent = `${S.W} × ${S.H}`;
    setDirty(false);
  } catch (e) {
    $('#stage-msg').textContent = errorText(e);
    $('#stage-msg').hidden = false;
  } finally {
    document.body.classList.remove('loading');
  }
  renderChrome();
  layout();
}

// ───────────────────────────── wiring ─────────────────────────────

function wire() {
  const win = currentWindow();
  $('#btn-min').onclick = () => win?.minimize();
  $('#btn-max').onclick = () => win?.toggleMaximize();
  $('#btn-close').onclick = async () => {
    // Marks stay editable later, so closing simply keeps them.
    if (S.dirty && !(await save({ quiet: true }))) return;
    win?.close();
  };
  $('#btn-save').onclick = () => save();
  $('#btn-copy').onclick = copy;
  document.querySelectorAll('[data-tool]').forEach((b) => { b.onclick = () => setTool(b.dataset.tool); });
  document.querySelectorAll('[data-size]').forEach((b) => { b.onclick = () => applyStyle({ size: b.dataset.size }); });
  $('#undo').onclick = undo;
  $('#redo').onclick = redo;
  $('#delete').onclick = deleteSelected;

  const keys = { v: 'select', a: 'arrow', r: 'box', h: 'highlight', t: 'text', b: 'blur' };
  window.addEventListener('keydown', (ev) => {
    if (S.editing) return;
    const k = ev.key.toLowerCase();
    const ctrl = ev.ctrlKey || ev.metaKey;
    let handled = true;
    if (ctrl && k === 'z') ev.shiftKey ? redo() : undo();
    else if (ctrl && k === 'y') redo();
    else if (ctrl && k === 's') save();
    else if (ctrl && k === 'c') copy();
    else if (!ctrl && keys[k]) setTool(keys[k]);
    else if (!ctrl && ['1', '2', '3'].includes(k)) applyStyle({ size: { 1: 's', 2: 'm', 3: 'l' }[k] });
    else if (k === 'delete' || k === 'backspace') deleteSelected();
    else if (k === 'escape') { S.selected = -1; renderChrome(); draw(); }
    else handled = false;
    if (handled) ev.preventDefault();
  });
  new ResizeObserver(layout).observe(stage);
}

async function init() {
  hydrateIcons();
  buildSwatches();
  wire();
  setTool('arrow');
  await listen('shot-load', (id) => load(String(id)));
  const id = new URLSearchParams(location.search).get('id');
  if (id) await load(id);
  else {
    $('#stage-msg').textContent = 'Take a screenshot or open one from the library to annotate it.';
    $('#stage-msg').hidden = false;
  }
}

init().catch((e) => toast(`Annotate could not start: ${errorText(e)}`, 'error'));
