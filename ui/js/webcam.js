import { invoke, currentWindow } from './tauri.js';
import { hydrateIcons } from './icons.js';

hydrateIcons();

const video = document.getElementById('video');
const msg = document.getElementById('msg');
let stream = null;

function markSize(size) {
  document.querySelectorAll('[data-size]').forEach((b) => {
    b.setAttribute('aria-pressed', String(b.dataset.size === size));
  });
}

async function openCamera(deviceId) {
  const base = { width: { ideal: 720 }, height: { ideal: 720 }, frameRate: { ideal: 30 } };
  try {
    return await navigator.mediaDevices.getUserMedia({
      video: deviceId ? { ...base, deviceId: { exact: deviceId } } : base,
      audio: false,
    });
  } catch (e) {
    if (deviceId) return navigator.mediaDevices.getUserMedia({ video: base, audio: false });
    throw e;
  }
}

async function start() {
  const settings = await invoke('get_settings');
  markSize(settings?.webcamSize || 'm');
  try {
    stream = await openCamera(settings?.webcamDevice);
    video.srcObject = stream;
    msg.hidden = true;
  } catch (e) {
    msg.textContent = e?.name === 'NotAllowedError' ? 'Camera access is blocked' : 'Camera unavailable';
  }
}

document.querySelectorAll('[data-size]').forEach((b) => {
  b.onclick = async () => {
    markSize(b.dataset.size);
    await invoke('set_webcam_size', { size: b.dataset.size });
  };
});

document.getElementById('hide').onclick = () => {
  stream?.getTracks().forEach((t) => t.stop());
  currentWindow()?.close();
};

window.addEventListener('beforeunload', () => stream?.getTracks().forEach((t) => t.stop()));
window.addEventListener('contextmenu', (e) => e.preventDefault());

start();
