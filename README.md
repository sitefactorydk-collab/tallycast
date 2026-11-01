# Tallycast

A screen recorder in the spirit of ScreenPal, built with Rust and Tauri 2.
Pick a display or drag out an area, record with microphone, computer sound and a
webcam bubble, pause and resume, and get an MP4. Then trim and cut it in the editor.
Take screenshots and mark them up with arrows, boxes, highlights, text and blur.

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
- **Optional graphics-card recording (Windows).** Recording uses the processor
  (GDI capture + x264) by default. Under *Video › Encoder* you can switch to AMD (AMF),
  NVIDIA (NVENC) or Intel (Quick Sync) with Desktop Duplication capture; Tallycast then
  tests what works on your PC, checks the test recordings' picture, and falls back to
  the processor by itself if the graphics card path fails.
- **Screenshots with annotations.** *Screenshot* (or `Alt+Shift+S` from anywhere, on the
  display the mouse is on) freezes the screen first, like the Snipping Tool, so hover
  menus and tooltips stay in the picture; then you drag out an area on the frozen image
  (or press Enter for all of it). It opens in *Annotate*: arrows, boxes,
  highlights, text and blur, in six colours and three sizes, with select/move/delete and
  undo. *Save* writes the PNG next to your videos; *Copy* puts it on the clipboard. The
  untouched capture is kept, so a screenshot can be reopened and its marks changed later.
  Blur replaces the pixels in the saved PNG.
- **Global hotkeys.** `Alt+Shift+R` starts with your last choice or stops and saves.
  `Alt+Shift+P` pauses and resumes. `Alt+Shift+S` takes a screenshot of an area.
- **Library.** Finished recordings with thumbnails, length, size; trim and cut, play,
  show in folder, or delete.
- **Trim and cut editor.** A filmstrip and waveform timeline. Drag the blue brackets
  (or press `I` / `O`) to set where the video starts and ends; drag across the timeline
  and press `Delete` to cut out a part in the middle. Every edit lands on a frame and can
  be undone. Playback skips what you cut. Edits are saved as you go.
- **Lossless export.** The result is written as a new MP4 next to the original, which
  is never changed. Untouched stretches are copied as they are; only the frames between
  a cut and the next keyframe are re-encoded, so exports are fast and keep their quality.
  A "Re-encode everything" switch is there as a fallback.
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

### Publishing a new version (with automatic updates)

Tallycast checks this repository's latest GitHub Release at start-up (and twice a day
while open). When it is newer than the running version, a banner offers the installer;
*Settings › Check for updates* checks right away.

To publish a version, commit your changes, then run:

```powershell
npm.cmd run release -- 0.2.0
```

This sets the version everywhere, commits "Release v0.2.0", tags `v0.2.0` and pushes.
GitHub Actions (`.github/workflows/release.yml`) then builds the installer on Windows,
with FFmpeg bundled, and publishes the release with `Tallycast_0.2.0_x64-setup.exe`
attached. That takes about 10–20 minutes; follow it under the repository's *Actions* tab.
Version numbers must go up (0.2.0 → 0.2.1 or 0.3.0).

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
  editor.html           Trim-and-cut editor (js/editor.js, css/editor.css)
  shot.html             Screenshot annotation (js/shot.js, css/shot.css)
  js/tauri.js           invoke/listen wrapper with browser mocks
src-tauri/src/
  lib.rs                Tauri commands, plugins, global hotkeys
  flow.rs               Capture workflow and state machine
  recorder.rs           Recording session: segments, pause/resume, finish
  ffmpeg.rs             Finding FFmpeg and building its command lines
  hwenc.rs              Capture/encoder pipelines, probing what works, encoder settings
  engine.rs             Caches the probe result and picks the pipeline per recording
  dxgi.rs               Maps a monitor to its Desktop Duplication output (Windows)
  audio.rs              Mic + system-audio capture (cpal), streamed to FFmpeg
  wins.rs               Creating and placing the helper windows
  library.rs            Saved recordings and thumbnails
  editor.rs             Editor window commands: load, filmstrip, waveform, export
  shot.rs               Screenshots: grab, keep original + marks, save annotated PNG
  edit.rs               Edit model (in/out/cuts) and the smart-cut / re-encode export
  media.rs              Reading recordings: stream info, frame index, waveform, frames
  mediaserver.rs        Localhost range server that streams a recording to the player
  settings.rs           Persisted settings
```

**Video.** By default FFmpeg grabs the screen with the platform's grabber (`gdigrab`
on Windows) and encodes with x264 `veryfast`. When a graphics-card encoder is chosen, it
captures with Desktop Duplication (`ddagrab`) and encodes H.264 on the graphics card
(`h264_amf`, `h264_nvenc` or `h264_qsv`). With FFmpeg 8 and
AMF or NVENC the captured GPU textures go straight into the encoder; older builds
download frames first. When one is first chosen, Tallycast runs FFmpeg briefly to see
which encoders and capture methods really work, compares each test recording's picture
with a processor-encoded reference (a path can run without errors and still record black
frames), and caches the answer per FFmpeg version (*Settings › Test again* repeats it). The monitor's Desktop Duplication index is looked up through
DXGI. GPU recordings use no B-frames and a keyframe every two seconds, so the editor's
lossless cut works on them. macOS (`avfoundation`) and Linux (`x11grab`) always use x264.

**Audio.** Rust captures each source with `cpal` (system sound via WASAPI loopback on
Windows) and streams raw samples to FFmpeg over a localhost socket, where they are mixed.
Each feed is paced by the clock and fills silence when Windows delivers nothing, so the
audio never drifts from the video when no sound is playing.

**Pause.** Every stretch between start/resume and pause/stop is a separate Matroska
segment (which survives crashes). Stopping joins the segments into one MP4 without
re-encoding.

**Editing.** The editor plays the original file through a tiny HTTP server bound to
127.0.0.1 (only registered files are served). On export, FFmpeg lists every frame and
keyframe without decoding. Each kept stretch is split into whole keyframe-to-keyframe
groups, which are stream-copied, and the few frames before the first and after the last
keyframe, which are re-encoded with x264. The pieces are joined as MPEG-TS (which carries
the H.264 settings with every keyframe) and muxed to MP4; sound is cut to the same frame
times and re-encoded as one AAC track, so it stays in sync.

**Staying out of the shot.** The control bar, countdown and area outline are marked
"exclude from capture" (Windows 10 2004+ and macOS), so they never appear in videos.
The webcam bubble deliberately does.

## Known limits in 0.1

- System sound recording is Windows-only for now. On macOS it needs a virtual audio
  device (for example BlackHole); on Linux it is not wired up yet.
- Linux needs an X11 session; Wayland capture is not implemented.
- On macOS the display order of `avfoundation` is assumed to match the system's monitor
  order.
- Desktop Duplication captures monitors driven by the first graphics adapter; on PCs
  with two GPUs, monitors on the other one are captured with GDI (still GPU-encoded).

## Next steps (planned)

1. ~~Trim and cut editor~~ (done)
2. ~~Hardware encoding and Desktop Duplication capture on Windows~~ (done)
3. Screenshots with annotations
4. System sound on macOS (ScreenCaptureKit) and Linux (PipeWire)

## License

Tallycast is released under the [MIT License](LICENSE).

The Windows installer bundles a separate FFmpeg program, which keeps its own license
(GPL for the Gyan builds). See "Build the installer" above before distributing it.
