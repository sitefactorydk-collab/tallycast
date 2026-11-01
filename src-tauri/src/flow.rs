//! The capture workflow: choose source → (select area) → countdown → record → save.
//!
//! Everything here blocks (process spawning, device opening, window bookkeeping), so
//! callers run these functions on a blocking thread, never on the UI thread.

use crate::hwenc::Plan;
use crate::library::{self, Recording};
use crate::recorder::{CaptureRect, RecordConfig, Session};
use crate::settings::{self, Settings};
use crate::wins::{self, MonitorInfo};
use crate::{audio, engine, ffmpeg, util};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub const RECORD_HOTKEY: &str = "Alt+Shift+R";
pub const PAUSE_HOTKEY: &str = "Alt+Shift+P";
pub const SHOT_HOTKEY: &str = "Alt+Shift+S";

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    #[default]
    Idle,
    Selecting,
    Countdown,
    Recording,
    Saving,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Selecting => "selecting",
            Phase::Countdown => "countdown",
            Phase::Recording => "recording",
            Phase::Saving => "saving",
        }
    }
}

#[derive(Clone)]
struct Pending {
    monitor: MonitorInfo,
    rect: Option<CaptureRect>,
    area: bool,
    /// A screenshot rather than a recording.
    shot: bool,
    settings: Settings,
    ffmpeg: PathBuf,
}

#[derive(Default)]
struct Inner {
    phase: Phase,
    pending: Option<Pending>,
    session: Option<Session>,
}

#[derive(Default)]
pub struct AppState {
    inner: Mutex<Inner>,
}

fn lock(app: &AppHandle) -> MutexGuard<'_, Inner> {
    let state: &AppState = app.state::<AppState>().inner();
    state.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ───────────────────────────── environment ─────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub os: String,
    pub ffmpeg_path: Option<String>,
    pub ffmpeg_version: Option<String>,
    pub monitors: Vec<MonitorInfo>,
    pub mic_devices: Vec<String>,
    pub default_mic: Option<String>,
    pub system_audio_supported: bool,
    pub wayland: bool,
    pub output_dir: String,
    pub record_hotkey: String,
    pub pause_hotkey: String,
    pub shot_hotkey: String,
}

pub fn environment(app: &AppHandle) -> Result<Environment, String> {
    let s = settings::load(app);
    let ff = ffmpeg::find(&s);
    let version = ff.as_deref().and_then(ffmpeg::version);
    let wayland = cfg!(target_os = "linux")
        && std::env::var("XDG_SESSION_TYPE").map(|v| v.eq_ignore_ascii_case("wayland")).unwrap_or(false);
    Ok(Environment {
        os: std::env::consts::OS.into(),
        ffmpeg_path: ff.as_deref().map(util::path_str),
        ffmpeg_version: version,
        monitors: wins::monitors(app)?,
        mic_devices: audio::input_device_names(),
        default_mic: audio::default_input_name(),
        system_audio_supported: audio::loopback_supported(),
        wayland,
        output_dir: util::path_str(&settings::output_dir(app, &s)),
        record_hotkey: RECORD_HOTKEY.into(),
        pause_hotkey: PAUSE_HOTKEY.into(),
        shot_hotkey: SHOT_HOTKEY.into(),
    })
}

// ───────────────────────────── status ─────────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub phase: String,
    pub elapsed_ms: u64,
    pub paused: bool,
    pub width: u32,
    pub height: u32,
}

pub fn status(app: &AppHandle) -> Status {
    let g = lock(app);
    let (elapsed_ms, paused, rect) = match &g.session {
        Some(s) => (s.elapsed().as_millis() as u64, s.is_paused(), Some(s.rect())),
        None => (0, false, g.pending.as_ref().and_then(|p| p.rect)),
    };
    Status {
        phase: g.phase.as_str().into(),
        elapsed_ms,
        paused,
        width: rect.map(|r| r.width).unwrap_or(0),
        height: rect.map(|r| r.height).unwrap_or(0),
    }
}

// ───────────────────────────── capture flow ─────────────────────────────

/// Starts a capture. `mode` is "screen" (whole monitor) or "area" (drag to select).
pub fn begin_capture(app: &AppHandle, mode: &str, monitor: usize) -> Result<(), String> {
    begin(app, mode, monitor, false)
}

/// Takes a screenshot of the whole monitor, or of an area the user drags out.
pub fn begin_screenshot(app: &AppHandle, mode: &str, monitor: usize) -> Result<(), String> {
    begin(app, mode, monitor, true)
}

fn begin(app: &AppHandle, mode: &str, monitor: usize, shot: bool) -> Result<(), String> {
    let mut s = settings::load(app);
    let ff = ffmpeg::find(&s).ok_or("FFmpeg was not found. Install it, then try again.")?;
    if cfg!(target_os = "linux")
        && std::env::var("XDG_SESSION_TYPE").map(|v| v.eq_ignore_ascii_case("wayland")).unwrap_or(false)
    {
        return Err("Recording on Wayland is not supported yet. Log in with an X11 session to record.".into());
    }
    let monitors = wins::monitors(app)?;
    let mon = monitors
        .get(monitor)
        .or_else(|| monitors.iter().find(|m| m.primary))
        .or_else(|| monitors.first())
        .cloned()
        .ok_or("No display was found.")?;
    let area = mode == "area";

    {
        let mut g = lock(app);
        if g.phase != Phase::Idle {
            return Err("A recording is already in progress.".into());
        }
        g.phase = if area { Phase::Selecting } else { Phase::Countdown };
        g.pending =
            Some(Pending { monitor: mon.clone(), rect: None, area, shot, settings: s.clone(), ffmpeg: ff });
    }

    // Screenshots don't change what the record hotkey does next.
    if !shot {
        s.last_mode = if area { "area".into() } else { "screen".into() };
    }
    s.last_monitor = mon.index;
    let _ = settings::save(app, &s);

    wins::hide_main(app);
    thread::sleep(Duration::from_millis(250)); // let the main window disappear

    let result = if area {
        wins::open_overlay(app, &mon, shot)
    } else if shot {
        shoot(app, CaptureRect::from_monitor(&mon))
    } else {
        proceed(app, CaptureRect::from_monitor(&mon))
    };
    if let Err(e) = &result {
        abort(app, e);
    }
    result
}

/// Called by the overlay with a rectangle relative to the monitor, in physical pixels.
pub fn region_selected(app: &AppHandle, x: i32, y: i32, width: u32, height: u32) -> Result<(), String> {
    let (mon, shot) = {
        let g = lock(app);
        if g.phase != Phase::Selecting {
            return Err("No area selection is in progress.".into());
        }
        g.pending.as_ref().map(|p| (p.monitor.clone(), p.shot)).ok_or("No capture is pending.")?
    };
    wins::close(app, wins::OVERLAY);
    thread::sleep(Duration::from_millis(120));
    let rect = CaptureRect::within_monitor(&mon, x, y, width, height);
    let result = if shot { shoot(app, rect) } else { proceed(app, rect) };
    if let Err(e) = &result {
        abort(app, e);
    }
    result
}

/// Grabs the still, then opens it for annotation.
fn shoot(app: &AppHandle, rect: CaptureRect) -> Result<(), String> {
    let p = {
        let mut g = lock(app);
        g.phase = Phase::Saving;
        g.pending.clone()
    }
    .ok_or("No capture is pending.")?;
    // Let the overlay and main window finish disappearing.
    thread::sleep(Duration::from_millis(180));
    let result = crate::shot::take(app, &p.ffmpeg, rect, &p.monitor, &p.settings);
    reset(app);
    wins::show_main(app);
    let rec = result?;
    wins::open_shot(app, &rec.id)
}

/// After the area is known: show helpers, then count down (or start right away).
fn proceed(app: &AppHandle, rect: CaptureRect) -> Result<(), String> {
    let pending = {
        let mut g = lock(app);
        let Some(p) = g.pending.as_mut() else {
            return Err("No capture is pending.".into());
        };
        p.rect = Some(rect);
        let snapshot = p.clone();
        g.phase = Phase::Countdown;
        snapshot
    };
    let s = &pending.settings;
    let scale = pending.monitor.scale;

    if s.webcam_enabled {
        if let Err(e) = wins::open_webcam(app, &rect, scale, &s.webcam_size) {
            eprintln!("[tallycast] webcam bubble: {e}");
        }
    }
    if pending.area {
        if let Err(e) = wins::open_frame(app, &rect, scale) {
            eprintln!("[tallycast] area outline: {e}");
        }
    }
    if s.countdown > 0 {
        wins::open_countdown(app, &rect, &pending.monitor)
    } else {
        start_recording(app)
    }
}

/// Called by the countdown window when it reaches zero.
pub fn countdown_done(app: &AppHandle) -> Result<(), String> {
    if lock(app).phase != Phase::Countdown {
        return Ok(());
    }
    wins::close(app, wins::COUNTDOWN);
    thread::sleep(Duration::from_millis(150)); // keep the countdown out of the first frame
    start_recording(app)
}

fn start_recording(app: &AppHandle) -> Result<(), String> {
    let p = lock(app).pending.clone().ok_or("No capture is pending.")?;
    let rect = p.rect.ok_or("No capture area was chosen.")?;
    let s = &p.settings;

    // A missing microphone shouldn't block the recording: skip it and say so afterwards.
    let mut mic_enabled = s.mic_enabled;
    if mic_enabled && audio::default_input_name().is_none() && audio::input_device_names().is_empty() {
        mic_enabled = false;
        let _ = app.emit(
            "capture-warning",
            "No microphone was found, so this recording has no voice. Connect a microphone, \
             or check Windows Settings › Privacy & security › Microphone and allow desktop apps."
                .to_string(),
        );
    }

    if s.system_audio && cfg!(target_os = "windows") && !audio::loopback_supported() {
        let _ = app.emit(
            "capture-warning",
            "Computer sound was not recorded because Windows reports no speakers or headphones. \
             Check that an output device is enabled in Settings › System › Sound."
                .to_string(),
        );
    }

    let plan = engine::plan_for(app, &p.ffmpeg, s, &p.monitor);
    eprintln!("[tallycast] recording with {}", plan.describe());
    let config = |plan: Plan| RecordConfig {
        ffmpeg: p.ffmpeg.clone(),
        plan,
        rect,
        monitor: p.monitor.clone(),
        fps: s.fps_clamped(),
        crf: s.crf(),
        show_cursor: s.show_cursor,
        mic_enabled,
        mic_device: s.mic_device.clone(),
        system_audio: s.system_audio,
        temp_dir: std::env::temp_dir().join(format!("tallycast-{}", util::now_ms())),
    };
    // If the graphics card path fails (driver, FFmpeg build, display setup), step down:
    // frames via system memory, then the processor. Failed plans are not tried again
    // this session.
    let mut attempt = plan;
    let mut first_error: Option<String> = None;
    let started = loop {
        match Session::start(config(attempt)) {
            Ok(session) => break Ok(session),
            Err(e) => {
                eprintln!("[tallycast] {} failed: {e}", attempt.describe());
                match attempt.fallback() {
                    Some(next) => {
                        engine::mark_broken(app, attempt);
                        first_error.get_or_insert(e);
                        attempt = next;
                        eprintln!("[tallycast] trying {}", attempt.describe());
                    }
                    None => break Err(e),
                }
            }
        }
    };
    if let (Ok(_), Some(e)) = (&started, &first_error) {
        let _ = app.emit(
            "capture-warning",
            format!(
                "Recording with {} did not start, so this recording used {} instead. \
                 You can choose the encoder under Video.\n\n{e}",
                plan.describe(),
                attempt.describe()
            ),
        );
    }
    match started {
        Ok(session) => {
            {
                let mut g = lock(app);
                g.session = Some(session);
                g.phase = Phase::Recording;
            }
            if let Err(e) = wins::open_recbar(app, &p.monitor) {
                eprintln!("[tallycast] control bar: {e}");
            }
            let _ = app.emit("recording-started", status(app));
            Ok(())
        }
        Err(e) => {
            abort(app, &e);
            Err(e)
        }
    }
}

pub fn toggle_pause(app: &AppHandle) -> Result<bool, String> {
    let mut g = lock(app);
    if g.phase != Phase::Recording {
        return Err("Nothing is being recorded.".into());
    }
    let session = g.session.as_mut().ok_or("Nothing is being recorded.")?;
    if session.is_paused() {
        session.resume()?;
    } else {
        session.pause()?;
    }
    Ok(session.is_paused())
}

pub fn stop_recording(app: &AppHandle) -> Result<(), String> {
    let (session, pending) = {
        let mut g = lock(app);
        if g.phase != Phase::Recording {
            return Err("Nothing is being recorded.".into());
        }
        g.phase = Phase::Saving;
        (g.session.take(), g.pending.clone())
    };
    let (Some(session), Some(pending)) = (session, pending) else {
        reset(app);
        return Err("Nothing is being recorded.".into());
    };

    wins::close(app, wins::WEBCAM);
    wins::close(app, wins::FRAME);

    let rect = session.rect();
    let created = chrono::Local::now();
    let title = format!("Recording {}", created.format("%Y-%m-%d at %H.%M.%S"));
    let out = util::unique_path(settings::output_dir(app, &pending.settings).join(format!("{title}.mp4")));

    let result = session.finish(&out);
    wins::close(app, wins::RECBAR);
    reset(app);
    wins::show_main(app);

    match result {
        Ok(duration_sec) => {
            let rec = Recording {
                id: util::now_ms().to_string(),
                title: library::display_title(&out),
                path: util::path_str(&out),
                created_ms: created.timestamp_millis(),
                duration_sec,
                width: rect.width,
                height: rect.height,
                size_bytes: std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0),
                kind: "video".into(),
            };
            if let Ok(thumb) = library::thumb_path(app, &rec.id) {
                ffmpeg::thumbnail(&pending.ffmpeg, &out, &thumb);
            }
            library::add(app, rec.clone())?;
            let _ = app.emit("recording-saved", library::view(app, rec));
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Cancels whatever stage the capture is in and throws away anything recorded.
pub fn cancel(app: &AppHandle) {
    let session = {
        let mut g = lock(app);
        if g.phase == Phase::Saving {
            return;
        }
        g.session.take()
    };
    if let Some(s) = session {
        s.discard();
    }
    wins::close_helpers(app);
    reset(app);
    wins::show_main(app);
}

fn reset(app: &AppHandle) {
    let mut g = lock(app);
    g.phase = Phase::Idle;
    g.pending = None;
    g.session = None;
}

/// Something failed before or while starting: tidy up and bring the main window back.
/// The error itself is reported once, by `report`.
fn abort(app: &AppHandle, message: &str) {
    eprintln!("[tallycast] capture aborted: {message}");
    let session = lock(app).session.take();
    if let Some(s) = session {
        s.discard();
    }
    wins::close_helpers(app);
    reset(app);
    wins::show_main(app);
}

/// Sends a flow error to the main window as a `capture-error` event.
pub fn report(app: &AppHandle, result: Result<(), String>) {
    if let Err(e) = result {
        wins::show_main(app);
        let _ = app.emit("capture-error", e);
    }
}

/// Global hotkey: drag out an area to screenshot on the last-used display.
pub fn hotkey_screenshot(app: &AppHandle) {
    if lock(app).phase != Phase::Idle {
        return;
    }
    let s = settings::load(app);
    report(app, begin_screenshot(app, "area", s.last_monitor));
}

/// Global hotkey: start with the last-used source, or stop if recording.
pub fn hotkey_record(app: &AppHandle) {
    let phase = lock(app).phase;
    let result = match phase {
        Phase::Idle => {
            let s = settings::load(app);
            begin_capture(app, &s.last_mode, s.last_monitor)
        }
        Phase::Recording => stop_recording(app),
        Phase::Selecting | Phase::Countdown => {
            cancel(app);
            Ok(())
        }
        Phase::Saving => Ok(()),
    };
    report(app, result);
}

pub fn resize_webcam(app: &AppHandle, size: &str) -> Result<(), String> {
    let mut s = settings::load(app);
    s.webcam_size = size.to_string();
    settings::save(app, &s)?;
    wins::resize_webcam(app, size)
}

pub fn open_recording(app: &AppHandle, id: &str, reveal: bool) -> Result<(), String> {
    let rec = library::find(app, id).ok_or("That recording is no longer in the library.")?;
    let path = PathBuf::from(&rec.path);
    if reveal {
        util::reveal_path(&path)
    } else {
        util::open_path(&path)
    }
}

pub fn open_output_dir(app: &AppHandle) -> Result<(), String> {
    let dir = settings::output_dir(app, &settings::load(app));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    util::open_path(&dir)
}
