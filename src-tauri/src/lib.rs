//! Tallycast — a cross-platform screen recorder built on Tauri.

mod audio;
mod edit;
mod editor;
mod ffmpeg;
mod flow;
mod library;
mod media;
mod mediaserver;
mod recorder;
mod settings;
mod util;
mod wins;

use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

fn record_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::ALT | Modifiers::SHIFT), Code::KeyR)
}

fn pause_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::ALT | Modifiers::SHIFT), Code::KeyP)
}

/// Runs blocking work off the UI thread.
async fn blocking<T, F>(app: AppHandle, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(AppHandle) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || f(app)).await.map_err(|e| e.to_string())?
}

/// Capture-flow commands always resolve; failures arrive as a `capture-error` event,
/// because the window that called (overlay, countdown, bar) may already be closed.
async fn flow_step<F>(app: AppHandle, f: F) -> Result<(), String>
where
    F: FnOnce(&AppHandle) -> Result<(), String> + Send + 'static,
{
    blocking(app, move |app| {
        let result = f(&app);
        flow::report(&app, result);
        Ok(())
    })
    .await
}

// ───────────────────────────── settings & environment ─────────────────────────────

#[tauri::command]
async fn get_environment(app: AppHandle) -> Result<flow::Environment, String> {
    blocking(app, |app| flow::environment(&app)).await
}

#[tauri::command]
async fn get_settings(app: AppHandle) -> Result<settings::Settings, String> {
    blocking(app, |app| Ok(settings::load(&app))).await
}

#[tauri::command]
async fn save_settings(app: AppHandle, settings: settings::Settings) -> Result<(), String> {
    blocking(app, move |app| settings::save(&app, &settings)).await
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    blocking(app, |app| {
        let picked =
            app.dialog().file().set_title("Choose where recordings are saved").blocking_pick_folder();
        Ok(picked.and_then(|p| p.into_path().ok()).map(|p| util::path_str(&p)))
    })
    .await
}

#[tauri::command]
async fn pick_ffmpeg(app: AppHandle) -> Result<Option<String>, String> {
    blocking(app, |app| {
        let mut dialog = app.dialog().file().set_title("Locate the FFmpeg program");
        if cfg!(windows) {
            dialog = dialog.add_filter("FFmpeg", &["exe"]);
        }
        let picked = dialog.blocking_pick_file();
        Ok(picked.and_then(|p| p.into_path().ok()).map(|p| util::path_str(&p)))
    })
    .await
}

// ───────────────────────────── library ─────────────────────────────

#[tauri::command]
async fn list_recordings(app: AppHandle) -> Result<Vec<library::RecordingView>, String> {
    blocking(app, |app| Ok(library::list(&app))).await
}

#[tauri::command]
async fn delete_recording(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |app| library::delete(&app, &id)).await
}

#[tauri::command]
async fn open_recording(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |app| flow::open_recording(&app, &id, false)).await
}

#[tauri::command]
async fn reveal_recording(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |app| flow::open_recording(&app, &id, true)).await
}

#[tauri::command]
async fn open_output_dir(app: AppHandle) -> Result<(), String> {
    blocking(app, |app| flow::open_output_dir(&app)).await
}

// ───────────────────────────── editor ─────────────────────────────

#[tauri::command]
async fn open_editor(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |app| editor::open(&app, &id)).await
}

#[tauri::command]
async fn editor_load(app: AppHandle, id: String) -> Result<editor::EditorData, String> {
    blocking(app, move |app| editor::load(&app, &id)).await
}

#[tauri::command]
async fn editor_filmstrip(app: AppHandle, id: String, count: usize, height: u32) -> Result<(), String> {
    blocking(app, move |app| editor::filmstrip(&app, &id, count, height)).await
}

#[tauri::command]
async fn editor_waveform(app: AppHandle, id: String) -> Result<Option<editor::Waveform>, String> {
    blocking(app, move |app| editor::waveform(&app, &id)).await
}

#[tauri::command]
async fn editor_save_edits(app: AppHandle, id: String, edits: edit::Edits) -> Result<(), String> {
    blocking(app, move |app| editor::save_edits(&app, &id, &edits)).await
}

#[tauri::command]
async fn editor_export(
    app: AppHandle,
    id: String,
    edits: edit::Edits,
    reencode: bool,
) -> Result<editor::ExportResult, String> {
    blocking(app, move |app| editor::export(&app, &id, &edits, reencode)).await
}

#[tauri::command]
async fn editor_cancel_export(app: AppHandle) -> Result<(), String> {
    blocking(app, |app| {
        editor::cancel_export(&app);
        Ok(())
    })
    .await
}

// ───────────────────────────── capture flow ─────────────────────────────

#[tauri::command]
async fn begin_capture(app: AppHandle, mode: String, monitor: usize) -> Result<(), String> {
    flow_step(app, move |app| flow::begin_capture(app, &mode, monitor)).await
}

#[tauri::command]
async fn region_selected(app: AppHandle, x: i32, y: i32, width: u32, height: u32) -> Result<(), String> {
    flow_step(app, move |app| flow::region_selected(app, x, y, width, height)).await
}

#[tauri::command]
async fn countdown_done(app: AppHandle) -> Result<(), String> {
    flow_step(app, flow::countdown_done).await
}

#[tauri::command]
async fn toggle_pause(app: AppHandle) -> Result<bool, String> {
    blocking(app, |app| flow::toggle_pause(&app)).await
}

#[tauri::command]
async fn stop_recording(app: AppHandle) -> Result<(), String> {
    flow_step(app, flow::stop_recording).await
}

#[tauri::command]
async fn cancel_capture(app: AppHandle) -> Result<(), String> {
    blocking(app, |app| {
        flow::cancel(&app);
        Ok(())
    })
    .await
}

#[tauri::command]
async fn get_status(app: AppHandle) -> Result<flow::Status, String> {
    blocking(app, |app| Ok(flow::status(&app))).await
}

#[tauri::command]
async fn set_webcam_size(app: AppHandle, size: String) -> Result<(), String> {
    blocking(app, move |app| flow::resize_webcam(&app, &size)).await
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if !matches!(event.state(), ShortcutState::Pressed) {
                        return;
                    }
                    let app = app.clone();
                    if shortcut == &record_shortcut() {
                        let _ = tauri::async_runtime::spawn_blocking(move || flow::hotkey_record(&app));
                    } else if shortcut == &pause_shortcut() {
                        let _ = tauri::async_runtime::spawn_blocking(move || {
                            let _ = flow::toggle_pause(&app);
                        });
                    }
                })
                .build(),
        )
        .manage(flow::AppState::default())
        .manage(editor::EditorState::default())
        .setup(|app| {
            for shortcut in [record_shortcut(), pause_shortcut()] {
                if let Err(e) = app.global_shortcut().register(shortcut) {
                    // Another app may own the combination; recording still works from the UI.
                    eprintln!("[tallycast] could not register a global hotkey: {e}");
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_environment,
            get_settings,
            save_settings,
            pick_folder,
            pick_ffmpeg,
            list_recordings,
            delete_recording,
            open_recording,
            reveal_recording,
            open_output_dir,
            open_editor,
            editor_load,
            editor_filmstrip,
            editor_waveform,
            editor_save_edits,
            editor_export,
            editor_cancel_export,
            begin_capture,
            region_selected,
            countdown_done,
            toggle_pause,
            stop_recording,
            cancel_capture,
            get_status,
            set_webcam_size,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tallycast");
}
