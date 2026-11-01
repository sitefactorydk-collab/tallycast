import { invoke } from './tauri.js';

const overlay = document.getElementById('overlay');
const sel = document.getElementById('sel');
const size = document.getElementById('sel-size');
const confirmBar = document.getElementById('confirm');
const cx = document.getElementById('cx');
const cy = document.getElementById('cy');

const MIN = 24; // CSS px

// The same overlay picks the area for a screenshot.
if (new URLSearchParams(location.search).get('shot')) {
  document.querySelector('.hint strong').textContent = 'Drag to select the area to capture';
  document.querySelector('.hint span').textContent = 'Press Enter to capture the whole screen. Press Esc to cancel.';
  document.getElementById('go').innerHTML = 'Capture this area';
  document.title = 'Select an area to capture';
}
let start = null;
let rect = null; // { x, y, w, h } in CSS px
let busy = false;

const dpr = () => window.devicePixelRatio || 1;

function draw() {
  if (!rect) {
    overlay.classList.remove('has-selection');
    return;
  }
  overlay.classList.add('has-selection');
  Object.assign(sel.style, { left: `${rect.x}px`, top: `${rect.y}px`, width: `${rect.w}px`, height: `${rect.h}px` });
  const r = dpr();
  size.textContent = `${Math.round(rect.w * r) & ~1} × ${Math.round(rect.h * r) & ~1}`;
  sel.classList.toggle('flip', rect.y < 36);

  // Place the confirm bar under the selection, or above / inside when there is no room.
  const barW = confirmBar.offsetWidth || 260;
  const barH = confirmBar.offsetHeight || 48;
  let top = rect.y + rect.h + 12;
  if (top + barH > window.innerHeight - 8) top = rect.y - barH - 12;
  if (top < 8) top = rect.y + rect.h - barH - 12;
  const left = Math.min(Math.max(8, rect.x + rect.w / 2 - barW / 2), window.innerWidth - barW - 8);
  Object.assign(confirmBar.style, { left: `${left}px`, top: `${top}px` });
}

function moveCrosshair(e) {
  cx.style.top = `${e.clientY}px`;
  cy.style.left = `${e.clientX}px`;
}

overlay.addEventListener('pointermove', (e) => {
  moveCrosshair(e);
  if (!start) return;
  const x = Math.min(start.x, e.clientX);
  const y = Math.min(start.y, e.clientY);
  rect = { x, y, w: Math.abs(e.clientX - start.x), h: Math.abs(e.clientY - start.y) };
  draw();
});

overlay.addEventListener('pointerdown', (e) => {
  if (e.button !== 0 || confirmBar.contains(e.target)) return;
  overlay.setPointerCapture(e.pointerId);
  start = { x: e.clientX, y: e.clientY };
  rect = null;
  overlay.classList.add('dragging');
  draw();
});

overlay.addEventListener('pointerup', () => {
  if (!start) return;
  start = null;
  overlay.classList.remove('dragging');
  if (!rect || rect.w < MIN || rect.h < MIN) rect = null;
  draw();
});

async function submit(r) {
  if (busy) return;
  busy = true;
  const k = dpr();
  await invoke('region_selected', {
    x: Math.round(r.x * k),
    y: Math.round(r.y * k),
    width: Math.round(r.w * k),
    height: Math.round(r.h * k),
  });
}

function cancel() {
  if (busy) return;
  busy = true;
  invoke('cancel_capture');
}

document.getElementById('go').onclick = () => rect && submit(rect);
document.getElementById('cancel').onclick = cancel;

window.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    if (rect) { rect = null; draw(); } else cancel();
  } else if (e.key === 'Enter') {
    submit(rect || { x: 0, y: 0, w: window.innerWidth, h: window.innerHeight });
  }
});

window.addEventListener('contextmenu', (e) => e.preventDefault());
window.focus();
