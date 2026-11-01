// Thin wrapper over the Tauri globals (withGlobalTauri = true).
// Outside Tauri (a plain browser), calls fall back to mock data so the UI can be
// previewed and styled with any static file server.

const T = window.__TAURI__;
export const isTauri = Boolean(T && T.core);

export async function invoke(cmd, args = {}) {
  if (isTauri) return T.core.invoke(cmd, args);
  return mock(cmd, args);
}

export async function listen(event, handler) {
  if (isTauri) return T.event.listen(event, (e) => handler(e.payload));
  return () => {};
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
  ffmpegPath: null, lastMode: 'screen', lastMonitor: 0,
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

function mock(cmd) {
  const now = Date.now();
  switch (cmd) {
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
        outputDir: 'C:\\Users\\you\\Videos\\Tallycast', recordHotkey: 'Alt+Shift+R', pauseHotkey: 'Alt+Shift+P',
      };
    case 'get_settings': return { ...mockSettings };
    case 'list_recordings':
      return [
        { id: '1', title: 'Recording 2026-11-01 at 10.42.17', path: 'C:\\Users\\you\\Videos\\Tallycast\\Recording 2026-11-01 at 10.42.17.mp4', createdMs: now - 40 * 60_000, durationSec: 312.4, width: 2560, height: 1440, sizeBytes: 48_200_000, thumbnail: mockThumb(220) },
        { id: '2', title: 'Recording 2026-10-31 at 16.05.51', path: '', createdMs: now - day - 3_600_000, durationSec: 74.9, width: 1280, height: 720, sizeBytes: 9_600_000, thumbnail: mockThumb(150) },
        { id: '3', title: 'Recording 2026-10-28 at 09.12.03', path: '', createdMs: now - 4 * day, durationSec: 1265, width: 1920, height: 1080, sizeBytes: 211_000_000, thumbnail: mockThumb(30) },
      ];
    case 'get_status': return { phase: 'recording', elapsedMs: 83_400, paused: false, width: 1920, height: 1080 };
    default: return null;
  }
}
