import { invoke } from './tauri.js';
import { icon, hydrateIcons } from './icons.js';

hydrateIcons();

const bar = document.getElementById('bar');
const time = document.getElementById('time');
const pauseBtn = document.getElementById('pause');
const stopBtn = document.getElementById('stop');
const discardBtn = document.getElementById('discard');

let saving = false;
let busy = false;

function fmt(ms) {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${r}` : `${m}:${r}`;
}

function render(status) {
  if (!status || saving) return;
  time.textContent = fmt(status.elapsedMs);
  bar.classList.toggle('paused', status.paused);
  pauseBtn.innerHTML = `${icon(status.paused ? 'resume' : 'pause')}<span>${status.paused ? 'Resume' : 'Pause'}</span>`;
  pauseBtn.title = status.paused ? 'Resume (Alt+Shift+P)' : 'Pause (Alt+Shift+P)';
}

async function poll() {
  try {
    render(await invoke('get_status'));
  } catch {
    /* window is closing */
  }
}

pauseBtn.onclick = async () => {
  if (busy || saving) return;
  busy = true;
  pauseBtn.disabled = true;
  try {
    await invoke('toggle_pause');
  } catch (e) {
    console.error(e);
  } finally {
    busy = false;
    pauseBtn.disabled = false;
    poll();
  }
};

stopBtn.onclick = () => {
  if (saving) return;
  saving = true;
  bar.classList.add('saving');
  bar.classList.remove('paused');
  stopBtn.innerHTML = '<span class="spinner" aria-hidden="true"></span><span>Saving</span>';
  stopBtn.style.opacity = '1';
  invoke('stop_recording'); // the window is closed by the app once the file is written
};

// Two-step discard so a stray click doesn't lose a recording.
let armTimer = 0;
discardBtn.onclick = () => {
  if (saving) return;
  if (!discardBtn.classList.contains('armed')) {
    discardBtn.classList.add('armed');
    discardBtn.innerHTML = `${icon('trash')}<span>Discard?</span>`;
    armTimer = setTimeout(() => {
      discardBtn.classList.remove('armed');
      discardBtn.innerHTML = icon('trash');
    }, 3000);
    return;
  }
  clearTimeout(armTimer);
  saving = true;
  invoke('cancel_capture');
};

window.addEventListener('contextmenu', (e) => e.preventDefault());
poll();
setInterval(poll, 250);
