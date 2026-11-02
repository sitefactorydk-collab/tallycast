//! User settings, persisted as JSON in the app's config folder.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Folder for finished recordings. `None` means `Videos/Tallycast`.
    pub output_dir: Option<String>,
    pub fps: u32,
    /// "compact" | "standard" | "high"
    pub quality: String,
    pub show_cursor: bool,
    /// Seconds of countdown before recording starts (0 = none).
    pub countdown: u32,
    pub mic_enabled: bool,
    /// Microphone device name. `None` means the system default.
    pub mic_device: Option<String>,
    pub system_audio: bool,
    pub webcam_enabled: bool,
    /// Browser `deviceId` of the webcam (the bubble is a web view).
    pub webcam_device: Option<String>,
    /// "s" | "m" | "l"
    pub webcam_size: String,
    /// Explicit path to ffmpeg; `None` means auto-detect.
    pub ffmpeg_path: Option<String>,
    /// "screen" | "area" — used by the global hotkey.
    pub last_mode: String,
    pub last_monitor: usize,
    /// Video encoder: "auto" | "amf" | "nvenc" | "qsv" | "x264".
    pub encoder: String,
    /// Screen capture on Windows: "auto" | "ddagrab" | "gdigrab".
    pub capture: String,
    /// A soft circle that follows the mouse pointer in recordings (Windows).
    pub cursor_halo: bool,
    /// Halo colour as "#rrggbb".
    pub halo_color: String,
    /// A ripple around the pointer on every click in recordings (Windows).
    pub click_effects: bool,
    /// Ripple colour as "#rrggbb".
    pub click_color: String,
    /// Size of the halo and ripples: "s" | "m" | "l".
    pub cursor_fx_size: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            output_dir: None,
            fps: 30,
            quality: "standard".into(),
            show_cursor: true,
            countdown: 3,
            mic_enabled: true,
            mic_device: None,
            system_audio: cfg!(target_os = "windows"),
            webcam_enabled: false,
            webcam_device: None,
            webcam_size: "m".into(),
            ffmpeg_path: None,
            last_mode: "screen".into(),
            last_monitor: 0,
            encoder: "auto".into(),
            capture: "auto".into(),
            cursor_halo: false,
            halo_color: "#ffd60a".into(),
            click_effects: false,
            click_color: "#e23b3e".into(),
            cursor_fx_size: "m".into(),
        }
    }
}

impl Settings {
    /// x264 constant rate factor for the chosen quality.
    pub fn crf(&self) -> u32 {
        match self.quality.as_str() {
            "high" => 18,
            "compact" => 28,
            _ => 23,
        }
    }

    pub fn fps_clamped(&self) -> u32 {
        self.fps.clamp(10, 60)
    }
}

fn settings_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("settings.json"))
}

pub fn load(app: &AppHandle) -> Settings {
    settings_file(app)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_file(app).ok_or("Could not locate the settings folder")?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

pub fn output_dir(app: &AppHandle, settings: &Settings) -> PathBuf {
    if let Some(dir) = settings.output_dir.as_ref().filter(|d| !d.trim().is_empty()) {
        return PathBuf::from(dir);
    }
    app.path()
        .video_dir()
        .or_else(|_| app.path().home_dir())
        .map(|d| d.join("Tallycast"))
        .unwrap_or_else(|_| std::env::temp_dir().join("Tallycast"))
}
