import { invoke } from './tauri.js';

const num = document.getElementById('num');
const ring = document.getElementById('ring');
const circumference = 2 * Math.PI * 78;
ring.style.strokeDasharray = `${circumference}`;

let done = false;
let timer = 0;

function finish() {
  if (done) return;
  done = true;
  clearInterval(timer);
  invoke('countdown_done');
}

function cancel() {
  if (done) return;
  done = true;
  clearInterval(timer);
  invoke('cancel_capture');
}

async function run() {
  const settings = await invoke('get_settings');
  const total = Math.max(1, Number(settings?.countdown) || 3);
  const startedAt = performance.now();
  let shown = 0;

  const frame = () => {
    if (done) return;
    const elapsed = (performance.now() - startedAt) / 1000;
    const left = Math.ceil(total - elapsed);
    if (left <= 0) return finish();
    if (left !== shown) {
      shown = left;
      num.textContent = String(left);
      num.classList.remove('tick');
      void num.offsetWidth; // restart the animation
      num.classList.add('tick');
    }
    ring.style.strokeDashoffset = `${circumference * (elapsed / total)}`;
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
  // rAF pauses in background windows; this guarantees the countdown still ends.
  timer = setInterval(() => {
    if ((performance.now() - startedAt) / 1000 >= total) finish();
  }, 200);
}

document.getElementById('face').onclick = finish;
window.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') cancel();
  if (e.key === 'Enter' || e.key === ' ') finish();
});

run();
