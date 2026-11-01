//! Screenshots: grab a still, keep the untouched original and the annotations in the
//! app's data folder, and write the annotated PNG the user sees next to their videos.
//!
//! Keeping the original means a screenshot can be reopened and its arrows, text and
//! blurs changed later; the blur in the exported PNG is real (pixels are replaced).

use crate::library::{self, Recording, RecordingView};
use crate::recorder::CaptureRect;
use crate::settings::{self, Settings};
use crate::wins::MonitorInfo;
use crate::{editor, ffmpeg, util};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("shots");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn original_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join(format!("{id}.png")))
}

fn annotations_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join(format!("{id}.json")))
}

/// Removes the original and annotations of a deleted screenshot.
pub fn forget(app: &AppHandle, id: &str) {
    if let Ok(p) = original_path(app, id) {
        let _ = fs::remove_file(p);
    }
    if let Ok(p) = annotations_path(app, id) {
        let _ = fs::remove_file(p);
    }
}

/// Reads width and height from a PNG header.
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((w, h))
}

fn refresh(app: &AppHandle, ff: &Path, rec: &Recording) {
    if let Ok(thumb) = library::thumb_path(app, &rec.id) {
        // A still has no frame at 0.5 s, so the old thumbnail would survive; start clean.
        let _ = fs::remove_file(&thumb);
        ffmpeg::thumbnail(ff, Path::new(&rec.path), &thumb);
    }
}

/// Grabs `rect` and adds it to the library. Returns the new entry.
pub fn take(
    app: &AppHandle,
    ff: &Path,
    rect: CaptureRect,
    monitor: &MonitorInfo,
    s: &Settings,
    frozen: Option<&Path>,
) -> Result<Recording, String> {
    let created = chrono::Local::now();
    let id = util::now_ms().to_string();
    let original = original_path(app, &id)?;
    match frozen {
        // The whole display was grabbed the moment the screenshot began; cut the area
        // out of that, so hover menus and tooltips are exactly as they were.
        Some(f) if rect == crate::recorder::CaptureRect::from_monitor(monitor) => {
            fs::copy(f, &original).map_err(|e| format!("Could not save the screenshot ({e})."))?;
        }
        Some(f) => ffmpeg::crop_png(
            ff,
            f,
            rect.x - monitor.x,
            rect.y - monitor.y,
            rect.width,
            rect.height,
            &original,
        )?,
        None => ffmpeg::screenshot(ff, rect, monitor, &original)?,
    }

    let dir = settings::output_dir(app, s);
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create the output folder ({e})."))?;
    let title = format!("Screenshot {}", created.format("%Y-%m-%d at %H.%M.%S"));
    let out = util::unique_path(dir.join(format!("{title}.png")));
    fs::copy(&original, &out).map_err(|e| format!("Could not save the screenshot ({e})."))?;

    let (width, height) =
        fs::read(&original).ok().and_then(|b| png_size(&b)).unwrap_or((rect.width, rect.height));
    let rec = Recording {
        id,
        title: library::display_title(&out),
        path: util::path_str(&out),
        created_ms: created.timestamp_millis(),
        duration_sec: 0.0,
        width,
        height,
        size_bytes: fs::metadata(&out).map(|m| m.len()).unwrap_or(0),
        kind: "image".into(),
    };
    refresh(app, ff, &rec);
    library::add(app, rec.clone())?;
    let _ = app.emit("recording-saved", library::view(app, rec.clone()));
    Ok(rec)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotData {
    pub recording: Recording,
    /// The untouched capture (or the PNG itself if the original is gone).
    pub image_url: String,
    /// Whether `image_url` is the untouched original; if not, annotations are baked in.
    pub original: bool,
    /// The annotation document saved last time, or null.
    pub annotations: Value,
}

pub fn load(app: &AppHandle, id: &str) -> Result<ShotData, String> {
    let rec = library::find(app, id).ok_or("That screenshot is no longer in the library.")?;
    if rec.kind != "image" {
        return Err("That item is a video, not a screenshot.".into());
    }
    let original = original_path(app, id)?;
    let has_original = original.exists();
    let image = if has_original { original } else { PathBuf::from(&rec.path) };
    if !image.exists() {
        return Err(format!("{} no longer exists.", rec.path));
    }
    let annotations = if has_original {
        annotations_path(app, id)
            .ok()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let image_url = editor::media_url(app, &image)?;
    Ok(ShotData { recording: rec, image_url, original: has_original, annotations })
}

/// Writes the annotated PNG over the screenshot's file and remembers the annotations.
pub fn save(
    app: &AppHandle,
    id: &str,
    png_base64: &str,
    annotations: &Value,
) -> Result<RecordingView, String> {
    let mut rec = library::find(app, id).ok_or("That screenshot is no longer in the library.")?;
    let bytes = util::base64_decode(png_base64)?;
    let (width, height) = png_size(&bytes).ok_or("The annotated image is not a valid PNG.")?;

    // Write next to the target first, so a failure never leaves half a file behind.
    let out = PathBuf::from(&rec.path);
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Could not create {} ({e}).", dir.display()))?;
    }
    let tmp = out.with_extension("png.part");
    fs::write(&tmp, &bytes).map_err(|e| format!("Could not save the screenshot ({e})."))?;
    fs::rename(&tmp, &out).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("Could not save the screenshot ({e}). Is it open in another program?")
    })?;

    if let Ok(p) = annotations_path(app, id) {
        if let Ok(json) = serde_json::to_string(annotations) {
            let _ = fs::write(p, json);
        }
    }
    rec.width = width;
    rec.height = height;
    rec.size_bytes = bytes.len() as u64;
    if let Some(ff) = ffmpeg::find(&settings::load(app)) {
        refresh(app, &ff, &rec);
    }
    library::update(app, rec.clone())?;
    let view = library::view(app, rec);
    let _ = app.emit("recording-saved", view.clone());
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_png_size() {
        let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        header.extend_from_slice(&3038u32.to_be_bytes());
        header.extend_from_slice(&1830u32.to_be_bytes());
        assert_eq!(png_size(&header), Some((3038, 1830)));
        assert_eq!(png_size(b"GIF89a"), None);
    }
}
