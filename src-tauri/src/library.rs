//! The list of finished recordings, kept as JSON next to their thumbnails.

use crate::util::{base64, path_str};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub id: String,
    pub title: String,
    pub path: String,
    pub created_ms: i64,
    pub duration_sec: f64,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub size_bytes: u64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RecordingView {
    #[serde(flatten)]
    pub rec: Recording,
    /// JPEG data URL, if a thumbnail exists.
    pub thumbnail: Option<String>,
}

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(dir.join("thumbs")).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn index_file(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("library.json"))
}

pub fn thumb_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("thumbs").join(format!("{id}.jpg")))
}

fn load(app: &AppHandle) -> Vec<Recording> {
    index_file(app)
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(app: &AppHandle, items: &[Recording]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(items).map_err(|e| e.to_string())?;
    fs::write(index_file(app)?, json).map_err(|e| e.to_string())
}

pub fn add(app: &AppHandle, rec: Recording) -> Result<(), String> {
    let mut items = load(app);
    items.retain(|r| r.path != rec.path);
    items.push(rec);
    save(app, &items)
}

fn thumb_data_url(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(format!("data:image/jpeg;base64,{}", base64(&bytes)))
}

pub fn view(app: &AppHandle, rec: Recording) -> RecordingView {
    let thumbnail = thumb_path(app, &rec.id).ok().and_then(|p| thumb_data_url(&p));
    RecordingView { rec, thumbnail }
}

/// Recordings whose files still exist, newest first. Missing files are pruned.
pub fn list(app: &AppHandle) -> Vec<RecordingView> {
    let items = load(app);
    let before = items.len();
    let mut kept: Vec<Recording> = items
        .into_iter()
        .filter_map(|mut r| {
            let meta = fs::metadata(&r.path).ok()?;
            r.size_bytes = meta.len();
            Some(r)
        })
        .collect();
    if kept.len() != before {
        let _ = save(app, &kept);
    }
    kept.sort_by(|a, b| b.created_ms.cmp(&a.created_ms));
    kept.into_iter().map(|r| view(app, r)).collect()
}

/// Removes a recording from the library and deletes its video file and thumbnail.
pub fn delete(app: &AppHandle, id: &str) -> Result<(), String> {
    let mut items = load(app);
    let Some(pos) = items.iter().position(|r| r.id == id) else {
        return Err("That recording is no longer in the library.".into());
    };
    let rec = items.remove(pos);
    if Path::new(&rec.path).exists() {
        fs::remove_file(&rec.path).map_err(|e| format!("Could not delete {} ({e}).", rec.path))?;
    }
    if let Ok(t) = thumb_path(app, id) {
        let _ = fs::remove_file(t);
    }
    save(app, &items)
}

pub fn find(app: &AppHandle, id: &str) -> Option<Recording> {
    load(app).into_iter().find(|r| r.id == id)
}

pub fn display_title(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path_str(path))
}
