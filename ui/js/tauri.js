// Thin wrapper over the Tauri globals (withGlobalTauri = true).
// Outside Tauri (a plain browser), calls fall back to mock data so the UI can be
// previewed and styled with any static file server.

const T = window.__TAURI__;
export const isTauri = Boolean(T && T.core);

export async function invoke(cmd, args = {}) {
  if (isTauri) return T.core.invoke(cmd, args);
  return mock(cmd, args);
}

const mockHandlers = {};

export async function listen(event, handler) {
  if (isTauri) return T.event.listen(event, (e) => handler(e.payload));
  (mockHandlers[event] ||= []).push(handler);
  return () => {};
}

function mockEmit(event, payload) {
  (mockHandlers[event] || []).forEach((h) => h(payload));
}

export function currentWindow() {
  return isTauri ? T.window.getCurrentWindow() : null;
}

// ───────────────────────────── browser preview mocks ─────────────────────────────

const day = 86_400_000;
const mockSettings = {
  outputDir: null, fps: 30, quality: 'standard', showCursor: true, countdown: 3,
  micEnabled: true, micDevice: null, systemAudio: true,
  webcamEnabled: false, webcamDevice: null, webcamSize: 'm',
  ffmpegPath: null, lastMode: 'screen', lastMonitor: 0, encoder: 'auto', capture: 'auto',
  cursorHalo: true, haloColor: '#ffd60a', clickEffects: true, clickColor: '#e23b3e', cursorFxSize: 'm',
};

function mockThumb(hue) {
  const c = document.createElement('canvas');
  c.width = 320; c.height = 180;
  const g = c.getContext('2d');
  g.fillStyle = `hsl(${hue} 30% 92%)`; g.fillRect(0, 0, 320, 180);
  g.fillStyle = `hsl(${hue} 35% 80%)`; g.fillRect(0, 0, 320, 22);
  g.fillStyle = `hsl(${hue} 45% 55%)`; g.fillRect(18, 40, 140, 10); g.fillRect(18, 58, 96, 8);
  g.fillStyle = `hsl(${hue} 20% 70%)`;
  for (let i = 0; i < 5; i++) g.fillRect(18, 82 + i * 16, 120 + ((i * 37) % 80), 6);
  g.fillStyle = `hsl(${hue} 40% 70%)`; g.fillRect(190, 40, 112, 120);
  return c.toDataURL('image/jpeg', 0.8);
}

function mockStripFrame(i, count) {
  const c = document.createElement('canvas');
  c.width = 104; c.height = 58;
  const g = c.getContext('2d');
  const hue = 210 + (i / count) * 40;
  g.fillStyle = `hsl(${hue} 25% 93%)`; g.fillRect(0, 0, 104, 58);
  g.fillStyle = `hsl(${hue} 30% 80%)`; g.fillRect(0, 0, 104, 8);
  g.fillStyle = `hsl(${hue} 50% 55%)`; g.fillRect(8, 16, 30 + ((i * 13) % 40), 5);
  g.fillStyle = `hsl(${hue} 15% 72%)`;
  for (let k = 0; k < 4; k++) g.fillRect(8, 27 + k * 7, 40 + ((i * 7 + k * 19) % 46), 3);
  g.fillStyle = `hsl(${(hue + 150) % 360} 45% 72%)`; g.fillRect(70, 16, 26, 34);
  return c.toDataURL('image/jpeg', 0.85);
}

function mockPeaks(seconds, perSec) {
  const n = Math.round(seconds * perSec);
  const out = new Array(n);
  let env = 0;
  for (let i = 0; i < n; i++) {
    const t = i / perSec;
    const speaking = Math.sin(t * 0.9) + Math.sin(t * 2.3 + 1) > -0.4;
    env += ((speaking ? 0.55 + 0.4 * Math.abs(Math.sin(t * 7.1)) : 0.03) - env) * 0.25;
    out[i] = Math.round(255 * Math.min(1, env * (0.6 + 0.4 * Math.abs(Math.sin(i * 1.7)))));
  }
  return out;
}

function mock(cmd, args) {
  const now = Date.now();
  switch (cmd) {
    case 'shot_load':
      return {
        recording: { id: args.id, title: 'Screenshot 2026-11-01 at 21.58.12', path: '', createdMs: now, durationSec: 0, width: 1600, height: 1000, sizeBytes: 820_000, kind: 'image' },
        imageUrl: 'mock/shot.png', original: true,
        annotations: { version: 1, shapes: [
          { type: 'highlight', x: 120, y: 236, w: 520, h: 46, color: '#f5b800', size: 'm' },
          { type: 'arrow', x1: 980, y1: 640, x2: 760, y2: 470, color: '#e23b3e', size: 'm' },
          { type: 'box', x: 690, y: 330, w: 420, h: 250, color: '#e23b3e', size: 'm' },
          { type: 'text', x: 990, y: 650, text: 'New export button', color: '#e23b3e', size: 'm' },
          { type: 'blur', x: 1240, y: 120, w: 300, h: 60, color: null, size: 'm' },
        ] },
      };
    case 'recording_url': return new URL('mock/shot.png', location.href).href;
    case 'shot_save': window.__lastSave = args; return { id: args.id };
    case 'editor_load':
      return {
        recording: { id: args.id, title: 'Recording 2026-11-01 at 10.42.17', path: 'C:\\Users\\you\\Videos\\Tallycast\\Recording 2026-11-01 at 10.42.17.mp4', createdMs: now, durationSec: 312.4, width: 2560, height: 1440, sizeBytes: 48_200_000 },
        info: { duration: 312.4, start: 0, width: 2560, height: 1440, fps: 30, videoStart: 0, videoCodec: 'h264', hasAudio: true },
        mediaUrl: 'mock/sample.webm',
        edits: { inPoint: 4.2, outPoint: 298.5, cuts: [{ start: 61.3, end: 74.9 }, { start: 182, end: 190.4 }] },
      };
    case 'editor_filmstrip':
      setTimeout(() => {
        for (let i = 0; i < args.count; i++) mockEmit('filmstrip-frame', { id: args.id, index: i, time: 0, src: mockStripFrame(i, args.count) });
      }, 150);
      return null;
    case 'editor_waveform':
      return { perSec: 100, peaks: mockPeaks(312.4, 100) };
    case 'editor_export':
      return new Promise((resolve) => {
        let f = 0;
        const timer = setInterval(() => {
          f = Math.min(1, f + 0.07);
          mockEmit('export-progress', { stage: 'exporting', fraction: f });
          if (f >= 1) {
            clearInterval(timer);
            resolve({
              recording: { id: 'x', title: 'Recording 2026-11-01 at 10.42.17 (edited)', path: '', createdMs: now, durationSec: 272.3, width: 2560, height: 1440, sizeBytes: 41_000_000 },
              report: { method: 'smart', duration: 272.3, reencoded: 6.4 },
            });
          }
        }, 120);
      });
    case 'get_environment':
      return {
        os: 'windows', ffmpegPath: 'C:\\Users\\you\\AppData\\Local\\Microsoft\\WinGet\\Links\\ffmpeg.exe',
        ffmpegVersion: 'ffmpeg version 7.1-essentials_build-www.gyan.dev',
        monitors: [
          { index: 0, name: '\\\\.\\DISPLAY1', x: 0, y: 0, width: 2560, height: 1440, scale: 1.25, primary: true },
          { index: 1, name: '\\\\.\\DISPLAY2', x: 2560, y: 180, width: 1920, height: 1080, scale: 1, primary: false },
        ],
        micDevices: ['Microphone (Shure MV7)', 'Headset Microphone (Jabra Evolve2 65)', 'Microphone Array (Realtek Audio)'],
        defaultMic: 'Microphone (Shure MV7)', systemAudioSupported: true, wayland: false,
        outputDir: 'C:\\Users\\you\\Videos\\Tallycast', recordHotkey: 'Alt+Shift+R', pauseHotkey: 'Alt+Shift+P', shotHotkey: 'Alt+Shift+S', appVersion: '0.1.0', repo: 'sitefactorydk-collab/tallycast',
      };
    case 'get_settings': return { ...mockSettings };
    case 'install_update':
      // Pretend to download a 31 MB installer; the real app then closes for the installer.
      return new Promise(() => {
        const total = 31_000_000;
        let got = 0;
        const timer = setInterval(() => {
          got = Math.min(total, got + 2_400_000);
          mockEmit('update-progress', { downloaded: got, total });
          if (got >= total) { clearInterval(timer); mockEmit('update-installing', null); }
        }, 120);
      });
    case 'get_engines':
      return {
        supported: true, ffmpegVersion: 'ffmpeg version 8.0-essentials_build-www.gyan.dev',
        encoders: [
          { id: 'amf', label: 'AMD graphics (AMF)', available: true },
          { id: 'nvenc', label: 'NVIDIA graphics (NVENC)', available: true },
          { id: 'qsv', label: 'Intel graphics (Quick Sync)', available: true },
          { id: 'x264', label: 'Processor (x264)', available: true },
        ],
        ddagrab: false, direct: [], tested: false,
        plan: { capture: 'gdigrab', encoder: 'x264', direct: false, shader: false, convert: false, outputIdx: 0 },
        planText: 'GDI capture + Processor (x264)',
      };
    case 'list_recordings':
      return [
        { id: '1', title: 'Recording 2026-11-01 at 10.42.17', path: 'C:\\Users\\you\\Videos\\Tallycast\\Recording 2026-11-01 at 10.42.17.mp4', createdMs: now - 40 * 60_000, durationSec: 312.4, width: 2560, height: 1440, sizeBytes: 48_200_000, thumbnail: mockThumb(220) },
        { id: '2', title: 'Recording 2026-10-31 at 16.05.51', path: '', createdMs: now - day - 3_600_000, durationSec: 74.9, width: 1280, height: 720, sizeBytes: 9_600_000, thumbnail: mockThumb(150) },
        { id: '4', title: 'Screenshot 2026-11-01 at 09.40.55', path: '', createdMs: now - 2 * 3_600_000, durationSec: 0, width: 1600, height: 1000, sizeBytes: 820_000, kind: 'image', thumbnail: mockThumb(260) },
        { id: '3', title: 'Recording 2026-10-28 at 09.12.03', path: '', createdMs: now - 4 * day, durationSec: 1265, width: 1920, height: 1080, sizeBytes: 211_000_000, thumbnail: mockThumb(30) },
      ];
    case 'get_status': return { phase: 'recording', elapsedMs: 83_400, paused: false, width: 1920, height: 1080 };
    default: return null;
  }
}
