# Tallycast

A screen recorder in the spirit of ScreenPal, built with Rust and Tauri 2.
Version 0.1 is the **recorder**: pick a display or drag out an area, record with
microphone, computer sound and a webcam bubble, pause and resume, and get an MP4.

Built for Windows, macOS and Linux, with Windows 11 as the first tested target.

## What it does

- **Full screen or area.** Choose a display on a to-scale map of your monitors, or
  drag out any rectangle. Mixed-DPI multi-monitor setups are handled in physical pixels.
- **Sound.** Any microphone, plus everything your computer plays (Windows), mixed into
  one track.
- **Webcam bubble.** A round, draggable camera bubble that floats over your screen
  and is recorded as part of the video. Three sizes.
- **Countdown, pause, resume, discard.** A floating control bar that is hidden from
  the recording itself.
- **Global hotkeys.** `Alt+Shift+R` starts with your last choice or stops and saves.
  `Alt+Shift+P` pauses and resumes.
- **Library.** Finished recordings with thumbnails, length, size; play, show in folder,
  or delete.
- Light and dark themes follow Windows.

Recordings are saved to `Videos\Tallycast` by default (change it in Settings).

---

## Set up on Windows 11

You need four things. Each is a one-time install.

1. **Rust** (with the MSVC toolchain)
   ```powershell
   winget install Rustlang.Rustup
   ```
   Rust on Windows also needs the **Visual Studio C++ Build Tools**. If you don't have
   Visual Studio, install the Build Tools and tick **"Desktop development with C++"**:
   ```powershell
   winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
   ```
2. **Node.js** (only used to run the Tauri command-line tool)
   ```powershell
   winget install OpenJS.NodeJS.LTS
   ```
3. **FFmpeg** (does the screen capture and video encoding)
   ```powershell
   winget install Gyan.FFmpeg
   ```
   Tallycast finds winget's copy automatically, even before your PATH refreshes.
4. **WebView2** is already part of Windows 11.

Close and reopen your terminal after installing so the new tools are on your PATH.

## Run it

From this folder:

```powershell
npm install
npm run dev
```

The first build compiles all the Rust dependencies and takes a few minutes; later runs
start in seconds.

### Build the installer

```powershell
npm run build
```

This produces one file you can copy to any Windows 10/11 PC:

```
src-tauri\target\release\bundle\nsis\Tallycast_0.1.0_x64-setup.exe
```

- FFmpeg is bundled inside the installer, so the target PC needs nothing else.
  Before each build, `scripts\prepare-ffmpeg.ps1` copies your installed `ffmpeg.exe`
  into `src-tauri\binaries\`. To bundle a different copy, set
  `$env:TALLYCAST_FFMPEG = 'C:\path\to\ffmpeg.exe'` and delete that folder first.
- It installs per user (no administrator prompt) and adds a Start menu entry and an
  uninstaller.
- The release build is fully optimised and takes several minutes the first time.
- Windows SmartScreen will warn about an unknown publisher until the installer is
  code-signed with a certificate.
- The Gyan FFmpeg builds are GPL-licensed. Bundling them is fine for your own use; if
  you distribute Tallycast publicly, you must comply with the GPL (or bundle an
  LGPL build of FFmpeg instead).

### First run checklist

- **Camera:** turn on *Webcam bubble* in the main window first. Windows may ask for
  camera permission; allow it there, in the big window, rather than in the small bubble.
  If nothing appears, check *Settings › Privacy & security › Camera* and allow desktop apps.
- **Microphone:** same idea, under *Privacy & security › Microphone*.
- **Hotkeys:** if `Alt+Shift+R` is already used by another app, Tallycast still records
  from its window; the console shows a warning.

### Previewing the interface without building

The UI is plain HTML, CSS and JavaScript in `ui/`. Outside Tauri it switches to mock
data, so you can style it in a browser:

```powershell
npm run preview
```

---

## How it works

```
ui/                     Front end (no bundler; Tauri serves this folder)
  index.html            Main window: source picker, sound, camera, video, library
  overlay.html          Full-screen area selector
  countdown.html        3-2-1 countdown
  recbar.html           Floating control bar (excluded from capture)
  webcam.html           Webcam bubble (included in capture)
  frame.html            Click-through outline around a recorded area
  js/tauri.js           invoke/listen wrapper with browser mocks
src-tauri/src/
  lib.rs                Tauri commands, plugins, global hotkeys
  flow.rs               Capture workflow and state machine
  recorder.rs           Recording session: segments, pause/resume, finish
  ffmpeg.rs             Finding FFmpeg and building its command lines
  audio.rs              Mic + system-audio capture (cpal), streamed to FFmpeg
  wins.rs               Creating and placing the helper windows
  library.rs            Saved recordings and thumbnails
  settings.rs           Persisted settings
```

**Video.** FFmpeg grabs the screen with the platform's grabber: `gdigrab` on Windows,
`avfoundation` on macOS, `x11grab` on Linux, and encodes H.264 (x264, `veryfast`).

**Audio.** Rust captures each source with `cpal` (system sound via WASAPI loopback on
Windows) and streams raw samples to FFmpeg over a localhost socket, where they are mixed.
Each feed is paced by the clock and fills silence when Windows delivers nothing, so the
audio never drifts from the video when no sound is playing.

**Pause.** Every stretch between start/resume and pause/stop is a separate Matroska
segment (which survives crashes). Stopping joins the segments into one MP4 without
re-encoding.

**Staying out of the shot.** The control bar, countdown and area outline are marked
"exclude from capture" (Windows 10 2004+ and macOS), so they never appear in videos.
The webcam bubble deliberately does.

## Known limits in 0.1

- System sound recording is Windows-only for now. On macOS it needs a virtual audio
  device (for example BlackHole); on Linux it is not wired up yet.
- Linux needs an X11 session; Wayland capture is not implemented.
- On macOS the display order of `avfoundation` is assumed to match the system's monitor
  order.
- `gdigrab` is CPU-based. For 4K at 60 fps a hardware path (`ddagrab` + NVENC/QSV/AMF)
  is the planned upgrade.

## Next steps (planned)

1. Trim and cut editor
2. Hardware encoding and Desktop Duplication capture on Windows
3. Screenshots with annotations
4. System sound on macOS (ScreenCaptureKit) and Linux (PipeWire)
