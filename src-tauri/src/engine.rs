//! Keeps track of which video engines work on this PC (see `hwenc`), probing once in
//! the background at start-up and caching the answer per FFmpeg binary and version.

use crate::hwenc::{self, Encoder, Engines, Plan};
use crate::settings::{self, Settings};
use crate::wins::MonitorInfo;
use crate::{dxgi, ffmpeg, util};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

#[derive(Default)]
pub struct EngineState {
    /// Held while probing, so concurrent callers wait for one probe.
    cache: Mutex<Option<Engines>>,
    /// Plans that failed to start in this session; they are not tried again.
    broken: Mutex<Vec<Plan>>,
}

pub fn mark_broken(app: &AppHandle, plan: Plan) {
    let state = app.state::<EngineState>();
    state.inner().broken.lock().unwrap_or_else(|p| p.into_inner()).push(plan);
}

fn state_broken_clear(app: &AppHandle) {
    let state = app.state::<EngineState>();
    state.inner().broken.lock().unwrap_or_else(|p| p.into_inner()).clear();
}

fn is_broken(app: &AppHandle, plan: &Plan) -> bool {
    let state = app.state::<EngineState>();
    let broken = state.inner().broken.lock().unwrap_or_else(|p| p.into_inner());
    broken.contains(plan)
}

fn cache_file(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join("engines.json"))
}

fn read_disk(app: &AppHandle) -> Option<Engines> {
    let text = fs::read_to_string(cache_file(app)?).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_disk(app: &AppHandle, e: &Engines) {
    if let (Some(file), Ok(json)) = (cache_file(app), serde_json::to_string_pretty(e)) {
        let _ = fs::write(file, json);
    }
}

fn cached_in(app: &AppHandle, slot: &mut Option<Engines>, path: &str, version: &str) -> Option<Engines> {
    let fresh =
        |e: &Engines| e.schema == hwenc::SCHEMA && e.ffmpeg_path == path && e.ffmpeg_version == version;
    if let Some(e) = slot.as_ref().filter(|e| fresh(e)) {
        return Some(e.clone());
    }
    let e = read_disk(app).filter(|e| fresh(e))?;
    *slot = Some(e.clone());
    Some(e)
}

/// The cached answer for `ffmpeg`, without probing.
fn cached(app: &AppHandle, ffmpeg_bin: &Path) -> Option<Engines> {
    let state = app.state::<EngineState>();
    let mut slot = state.inner().cache.lock().unwrap_or_else(|p| p.into_inner());
    let version = ffmpeg::version(ffmpeg_bin).unwrap_or_default();
    cached_in(app, &mut slot, &util::path_str(ffmpeg_bin), &version)
}

/// What works with `ffmpeg`. Probes (a few seconds) unless a cached answer for this
/// exact binary and version exists, or `force` is set.
pub fn engines(app: &AppHandle, ffmpeg_bin: &Path, force: bool) -> Engines {
    let state = app.state::<EngineState>();
    let mut slot = state.inner().cache.lock().unwrap_or_else(|p| p.into_inner());
    let path = util::path_str(ffmpeg_bin);
    let version = ffmpeg::version(ffmpeg_bin).unwrap_or_default();
    if !force {
        if let Some(e) = cached_in(app, &mut slot, &path, &version) {
            return e;
        }
    }
    let e = hwenc::probe(ffmpeg_bin, &version);
    eprintln!(
        "[tallycast] video engines: hardware encoders {:?}, desktop duplication {}, direct {:?}, shader {:?}, \
         converted {:?}",
        e.encoders, e.ddagrab, e.direct, e.shader, e.converted
    );
    for note in &e.notes {
        eprintln!("[tallycast]   not used: {note}");
    }
    write_disk(app, &e);
    *slot = Some(e.clone());
    e
}

/// Forgets the cached answer, e.g. after a GPU recording failed to start.
/// Starts the probe in the background so the first recording doesn't wait for it.
/// Only when the settings ask for the graphics card: the processor needs no testing.
pub fn warm_up(app: &AppHandle) {
    let s = settings::load(app);
    if !cfg!(windows) || !hwenc::wants_gpu(&s.encoder, &s.capture) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        if let Some(ff) = ffmpeg::find(&s) {
            engines(&app, &ff, false);
        }
    });
}

/// The pipeline to record `monitor` with, honouring the user's choices.
pub fn plan_for(app: &AppHandle, ffmpeg_bin: &Path, s: &Settings, monitor: &MonitorInfo) -> Plan {
    if !cfg!(windows) || !hwenc::wants_gpu(&s.encoder, &s.capture) {
        return Plan::safe();
    }
    let e = engines(app, ffmpeg_bin, false);
    let output = dxgi::output_index(monitor.x, monitor.y, monitor.width, monitor.height);
    let mut plan = hwenc::choose(&e, &s.encoder, &s.capture, output);
    // Skip what already failed this session, one step at a time.
    while is_broken(app, &plan) {
        match plan.fallback() {
            Some(next) => plan = next,
            None => break,
        }
    }
    plan
}

// ───────────────────────────── for the settings UI ─────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderOption {
    pub id: &'static str,
    pub label: &'static str,
    pub available: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineView {
    pub supported: bool,
    pub ffmpeg_version: String,
    pub encoders: Vec<EncoderOption>,
    pub ddagrab: bool,
    pub direct: Vec<Encoder>,
    /// False until the graphics card has been tested (only done when asked for).
    pub tested: bool,
    /// What the current choices use on the main display.
    pub plan: Plan,
    pub plan_text: String,
}

pub fn view(app: &AppHandle, force: bool) -> Result<EngineView, String> {
    let s = settings::load(app);
    let ff = ffmpeg::find(&s).ok_or("FFmpeg was not found.")?;
    if force {
        state_broken_clear(app);
    }
    let gpu = hwenc::wants_gpu(&s.encoder, &s.capture);
    let found = if !cfg!(windows) {
        None
    } else if force || gpu {
        Some(engines(app, &ff, force))
    } else {
        cached(app, &ff)
    };
    let tested = found.is_some();
    let e = found.unwrap_or_default();
    let monitors = crate::wins::monitors(app).unwrap_or_default();
    let mon = monitors.get(s.last_monitor).or_else(|| monitors.iter().find(|m| m.primary)).cloned();
    let plan = match mon {
        Some(m) if cfg!(windows) && gpu => {
            let output = dxgi::output_index(m.x, m.y, m.width, m.height);
            hwenc::choose(&e, &s.encoder, &s.capture, output)
        }
        _ => Plan::safe(),
    };
    let encoders = [Encoder::Amf, Encoder::Nvenc, Encoder::Qsv, Encoder::X264]
        .into_iter()
        .map(|enc| EncoderOption {
            id: enc.id(),
            label: enc.label(),
            // Untested encoders are offered; choosing one runs the test.
            available: enc == Encoder::X264 || !tested || e.encoders.contains(&enc),
        })
        .collect();
    Ok(EngineView {
        supported: cfg!(windows),
        ffmpeg_version: e.ffmpeg_version.clone(),
        encoders,
        ddagrab: e.ddagrab,
        direct: e.direct.clone(),
        tested,
        plan_text: plan.describe(),
        plan,
    })
}
