//! The trim-and-cut editor window: loading a recording, filmstrip and waveform for the
//! timeline, saving edits, and exporting the result to a new file.

use crate::edit::{self, Edits, ExportJob, ExportReport, Runner};
use crate::library::{self, Recording, RecordingView};
use crate::media::{self, MediaInfo};
use crate::mediaserver::MediaServer;
use crate::{ffmpeg, settings, util, wins};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Default)]
pub struct EditorState {
    server: Mutex<Option<Arc<MediaServer>>>,
    export_cancel: Mutex<Option<Arc<AtomicBool>>>,
}

fn state(app: &AppHandle) -> &EditorState {
    app.state::<EditorState>().inner()
}

fn media_server(app: &AppHandle) -> Result<Arc<MediaServer>, String> {
    let mut slot = state(app).server.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(s) = slot.as_ref() {
        return Ok(s.clone());
    }
    let server =
        Arc::new(MediaServer::start().map_err(|e| format!("Could not start the video player ({e})."))?);
    *slot = Some(server.clone());
    Ok(server)
}

/// A localhost URL the web views can load `path` from (with byte ranges and CORS).
pub fn media_url(app: &AppHandle, path: &Path) -> Result<String, String> {
    Ok(media_server(app)?.url_for(path))
}

fn ffmpeg_path(app: &AppHandle) -> Result<PathBuf, String> {
    ffmpeg::find(&settings::load(app))
        .ok_or_else(|| "FFmpeg was not found. Install it, then try again.".into())
}

fn recording(app: &AppHandle, id: &str) -> Result<Recording, String> {
    let rec = library::find(app, id).ok_or("That recording is no longer in the library.")?;
    if !Path::new(&rec.path).exists() {
        return Err(format!("{} no longer exists.", rec.path));
    }
    Ok(rec)
}

fn cache_dir(app: &AppHandle, kind: &str) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join(kind);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Removes saved edits and cached filmstrip frames of a deleted recording.
pub fn forget(app: &AppHandle, id: &str) {
    if let Ok(d) = cache_dir(app, "edits") {
        let _ = fs::remove_file(d.join(format!("{id}.json")));
    }
    if let Ok(d) = cache_dir(app, "strips") {
        let _ = fs::remove_dir_all(d.join(id));
    }
}

// ───────────────────────────── window ─────────────────────────────

pub fn open(app: &AppHandle, id: &str) -> Result<(), String> {
    recording(app, id)?;
    wins::open_editor(app, id)
}

// ───────────────────────────── loading ─────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorData {
    pub recording: Recording,
    pub info: MediaInfo,
    pub media_url: String,
    pub edits: Edits,
}

pub fn load(app: &AppHandle, id: &str) -> Result<EditorData, String> {
    let rec = recording(app, id)?;
    let ff = ffmpeg_path(app)?;
    let path = PathBuf::from(&rec.path);
    let info = media::probe(&ff, &path)?;
    let media_url = media_server(app)?.url_for(&path);
    Ok(EditorData { edits: load_edits(app, id), recording: rec, info, media_url })
}

fn load_edits(app: &AppHandle, id: &str) -> Edits {
    cache_dir(app, "edits")
        .ok()
        .and_then(|d| fs::read_to_string(d.join(format!("{id}.json"))).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_edits(app: &AppHandle, id: &str, edits: &Edits) -> Result<(), String> {
    let file = cache_dir(app, "edits")?.join(format!("{id}.json"));
    let json = serde_json::to_string_pretty(edits).map_err(|e| e.to_string())?;
    fs::write(file, json).map_err(|e| e.to_string())
}

// ───────────────────────────── timeline data ─────────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StripFrame {
    id: String,
    index: usize,
    time: f64,
    src: String,
}

/// Emits `filmstrip-frame` events as frames become ready, `count` frames evenly spread
/// over the video, `height` pixels tall. Frames are cached on disk per recording.
pub fn filmstrip(app: &AppHandle, id: &str, count: usize, height: u32) -> Result<(), String> {
    let rec = recording(app, id)?;
    let ff = ffmpeg_path(app)?;
    let count = count.clamp(1, 240);
    let height = height.clamp(24, 240);
    let dir = cache_dir(app, "strips")?.join(id).join(format!("{count}x{height}"));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let src = PathBuf::from(&rec.path);
    let duration = rec.duration_sec.max(0.1);

    let next = AtomicUsize::new(0);
    let workers = thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 6);
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= count {
                    break;
                }
                // The middle of each slot, so the first tile isn't the countdown frame.
                let time = duration * (i as f64 + 0.5) / count as f64;
                let file = dir.join(format!("{i:03}.jpg"));
                let bytes = fs::read(&file).ok().or_else(|| {
                    let b = media::frame_jpeg(&ff, &src, time, height)?;
                    let _ = fs::write(&file, &b);
                    Some(b)
                });
                if let Some(b) = bytes {
                    let data_url = format!("data:image/jpeg;base64,{}", util::base64(&b));
                    let frame = StripFrame { id: id.to_string(), index: i, time, src: data_url };
                    let _ = app.emit_to(wins::EDITOR, "filmstrip-frame", frame);
                }
            });
        }
    });
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waveform {
    pub per_sec: u32,
    pub peaks: Vec<u8>,
}

pub const WAVE_PER_SEC: u32 = 100;

/// Peak levels for the timeline, or `None` when the recording has no sound.
pub fn waveform(app: &AppHandle, id: &str) -> Result<Option<Waveform>, String> {
    let rec = recording(app, id)?;
    let ff = ffmpeg_path(app)?;
    let path = PathBuf::from(&rec.path);
    if !media::probe(&ff, &path)?.has_audio {
        return Ok(None);
    }
    let peaks = media::waveform(&ff, &path, WAVE_PER_SEC)?;
    Ok(Some(Waveform { per_sec: WAVE_PER_SEC, peaks }))
}

// ───────────────────────────── export ─────────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    stage: &'static str,
    fraction: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub recording: RecordingView,
    pub report: ExportReport,
}

/// "Name.mp4" → "Name (edited).mp4" next to the original, never overwriting anything.
fn export_path(src: &Path) -> PathBuf {
    let stem = library::display_title(src);
    let stem = stem.strip_suffix(" (edited)").unwrap_or(&stem).to_string();
    let dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
    util::unique_path(dir.join(format!("{stem} (edited).mp4")))
}

pub fn export(app: &AppHandle, id: &str, edits: &Edits, reencode: bool) -> Result<ExportResult, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state(app).export_cancel.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_some() {
            return Err("An export is already running.".into());
        }
        *slot = Some(cancel.clone());
    }
    let result = run_export(app, id, edits, reencode, &cancel);
    *state(app).export_cancel.lock().unwrap_or_else(|p| p.into_inner()) = None;
    result
}

pub fn cancel_export(app: &AppHandle) {
    if let Some(c) = state(app).export_cancel.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        c.store(true, Ordering::SeqCst);
    }
}

fn run_export(
    app: &AppHandle,
    id: &str,
    edits: &Edits,
    reencode: bool,
    cancel: &AtomicBool,
) -> Result<ExportResult, String> {
    let emit = |stage: &'static str, fraction: f64| {
        let _ = app.emit_to(wins::EDITOR, "export-progress", Progress { stage, fraction });
    };
    emit("preparing", 0.0);

    let rec = recording(app, id)?;
    let s = settings::load(app);
    let ff = ffmpeg_path(app)?;
    let src = PathBuf::from(&rec.path);
    let info = media::probe(&ff, &src)?;
    let spans = edit::keep_spans(info.duration, edits);
    if spans.is_empty() {
        return Err("Nothing is left to export: every part of the video is cut.".into());
    }
    // The frame index lets both strategies cut on exact frames. Without it, re-encode
    // by time.
    let index = match media::index_video(&ff, &src, info.start) {
        Ok(i) => Some(i),
        Err(e) => {
            eprintln!("[tallycast] could not index {}: {e}", rec.path);
            None
        }
    };
    if cancel.load(Ordering::SeqCst) {
        return Err(edit::CANCELLED.into());
    }

    let out = export_path(&src);
    let work = edit::work_dir(&util::now_ms().to_string());
    // A little more quality than the recording, so re-encoded edges don't stand out.
    let crf = s.crf().saturating_sub(2).max(14);
    let job = ExportJob { src: &src, out: &out, work_dir: &work, info: &info, spans: &spans, crf };

    let mut last = -1.0;
    let mut progress = |f: f64| {
        if f - last >= 0.005 || f >= 1.0 {
            last = f;
            emit("exporting", f);
        }
    };
    let mut runner = Runner::new(&ff, cancel, &mut progress, 1.0);
    let report = edit::export(&job, index.as_ref(), reencode, &mut runner)?;
    emit("finishing", 1.0);

    let new = Recording {
        id: util::now_ms().to_string(),
        title: library::display_title(&out),
        path: util::path_str(&out),
        created_ms: chrono::Local::now().timestamp_millis(),
        duration_sec: report.duration,
        width: info.width,
        height: info.height,
        size_bytes: fs::metadata(&out).map(|m| m.len()).unwrap_or(0),
        kind: "video".into(),
    };
    if let Ok(thumb) = library::thumb_path(app, &new.id) {
        ffmpeg::thumbnail(&ff, &out, &thumb);
    }
    library::add(app, new.clone())?;
    let view = library::view(app, new);
    let _ = app.emit("recording-saved", view.clone());
    Ok(ExportResult { recording: view, report })
}
