//! Creation and placement of Tallycast's helper windows.
//!
//! All geometry here is in *physical* pixels in absolute desktop coordinates, which is
//! what both the OS and FFmpeg's screen grabbers use. Windows are built hidden, then
//! moved with physical coordinates so mixed-DPI multi-monitor setups land correctly.

use crate::recorder::CaptureRect;
use serde::Serialize;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

pub const MAIN: &str = "main";
pub const OVERLAY: &str = "overlay";
pub const COUNTDOWN: &str = "countdown";
pub const RECBAR: &str = "recbar";
pub const WEBCAM: &str = "webcam";
pub const FRAME: &str = "frame";

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub index: usize,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub primary: bool,
}

pub fn monitors(app: &AppHandle) -> Result<Vec<MonitorInfo>, String> {
    let win = app.get_webview_window(MAIN).ok_or("The main window is missing.")?;
    let list = win.available_monitors().map_err(|e| e.to_string())?;
    let primary = win.primary_monitor().ok().flatten();
    let primary_pos = primary.as_ref().map(|m| (m.position().x, m.position().y));

    let mut out: Vec<MonitorInfo> = list
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let pos = m.position();
            let size = m.size();
            MonitorInfo {
                index: i,
                name: m.name().cloned().unwrap_or_else(|| format!("Display {}", i + 1)),
                x: pos.x,
                y: pos.y,
                width: size.width,
                height: size.height,
                scale: m.scale_factor(),
                primary: primary_pos == Some((pos.x, pos.y)),
            }
        })
        .collect();
    if !out.iter().any(|m| m.primary) {
        if let Some(first) = out.first_mut() {
            first.primary = true;
        }
    }
    Ok(out)
}

pub fn hide_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.hide();
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn close(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.close();
    }
}

pub fn close_helpers(app: &AppHandle) {
    for label in [OVERLAY, COUNTDOWN, RECBAR, WEBCAM, FRAME] {
        close(app, label);
    }
}

/// Closing is asynchronous; wait until a label is free before reusing it.
fn ensure_closed(app: &AppHandle, label: &str) {
    if app.get_webview_window(label).is_none() {
        return;
    }
    close(app, label);
    let start = Instant::now();
    while app.get_webview_window(label).is_some() && start.elapsed() < Duration::from_millis(1500) {
        thread::sleep(Duration::from_millis(20));
    }
}

struct Spec<'a> {
    label: &'a str,
    url: &'a str,
    title: &'a str,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    scale: f64,
    /// Hide from screen capture (Windows 10 2004+ / macOS).
    protect: bool,
    focus: bool,
}

fn make(app: &AppHandle, s: Spec) -> Result<WebviewWindow, String> {
    ensure_closed(app, s.label);
    let scale = if s.scale > 0.0 { s.scale } else { 1.0 };
    let win = WebviewWindowBuilder::new(app, s.label, WebviewUrl::App(s.url.into()))
        .title(s.title)
        .inner_size(s.w as f64 / scale, s.h as f64 / scale)
        .position(s.x as f64 / scale, s.y as f64 / scale)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(s.focus)
        .visible(false)
        .content_protected(s.protect)
        .build()
        .map_err(|e| format!("Could not open the {} window ({e}).", s.label))?;
    let _ = win.set_size(PhysicalSize::new(s.w, s.h));
    let _ = win.set_position(PhysicalPosition::new(s.x, s.y));
    let _ = win.show();
    if s.focus {
        let _ = win.set_focus();
    }
    Ok(win)
}

/// Full-monitor transparent overlay where the user drags out an area.
pub fn open_overlay(app: &AppHandle, mon: &MonitorInfo) -> Result<(), String> {
    make(
        app,
        Spec {
            label: OVERLAY,
            url: "overlay.html",
            title: "Select an area",
            x: mon.x,
            y: mon.y,
            w: mon.width,
            h: mon.height,
            scale: mon.scale,
            protect: false,
            focus: true,
        },
    )
    .map(|_| ())
}

pub fn open_countdown(app: &AppHandle, rect: &CaptureRect, mon: &MonitorInfo) -> Result<(), String> {
    let size = (200.0 * mon.scale).round() as u32;
    let x = rect.x + rect.width as i32 / 2 - size as i32 / 2;
    let y = rect.y + rect.height as i32 / 2 - size as i32 / 2;
    make(
        app,
        Spec {
            label: COUNTDOWN,
            url: "countdown.html",
            title: "Starting",
            x,
            y,
            w: size,
            h: size,
            scale: mon.scale,
            protect: true,
            focus: true,
        },
    )
    .map(|_| ())
}

/// The floating control bar shown while recording. Excluded from the capture.
pub fn open_recbar(app: &AppHandle, mon: &MonitorInfo) -> Result<(), String> {
    let w = (372.0 * mon.scale).round() as u32;
    let h = (64.0 * mon.scale).round() as u32;
    let x = mon.x + mon.width as i32 / 2 - w as i32 / 2;
    let y = mon.y + mon.height as i32 - h as i32 - (88.0 * mon.scale).round() as i32;
    make(
        app,
        Spec {
            label: RECBAR,
            url: "recbar.html",
            title: "Recording",
            x,
            y,
            w,
            h,
            scale: mon.scale,
            protect: true,
            focus: false,
        },
    )
    .map(|_| ())
}

/// A click-through outline drawn just outside the recorded area.
pub fn open_frame(app: &AppHandle, rect: &CaptureRect, scale: f64) -> Result<(), String> {
    let pad = (4.0 * scale).round().max(2.0) as i32;
    let win = make(
        app,
        Spec {
            label: FRAME,
            url: "frame.html",
            title: "Recording area",
            x: rect.x - pad,
            y: rect.y - pad,
            w: rect.width + 2 * pad as u32,
            h: rect.height + 2 * pad as u32,
            scale,
            protect: true,
            focus: false,
        },
    )?;
    let _ = win.set_ignore_cursor_events(true);
    Ok(())
}

pub fn webcam_diameter(size: &str) -> f64 {
    match size {
        "s" => 150.0,
        "l" => 300.0,
        _ => 210.0,
    }
}

/// The webcam bubble. Deliberately *not* excluded from capture: it is part of the video.
pub fn open_webcam(app: &AppHandle, rect: &CaptureRect, scale: f64, size: &str) -> Result<(), String> {
    let d = (webcam_diameter(size) * scale).round() as u32;
    let margin = (28.0 * scale).round() as i32;
    let x = rect.x + margin;
    let y = rect.y + rect.height as i32 - d as i32 - margin;
    make(
        app,
        Spec {
            label: WEBCAM,
            url: "webcam.html",
            title: "Webcam",
            x,
            y,
            w: d,
            h: d,
            scale,
            protect: false,
            focus: false,
        },
    )
    .map(|_| ())
}

/// Resizes the bubble while keeping its bottom-left corner in place.
pub fn resize_webcam(app: &AppHandle, size: &str) -> Result<(), String> {
    let win = app.get_webview_window(WEBCAM).ok_or("The webcam bubble is not open.")?;
    let scale = win.scale_factor().unwrap_or(1.0);
    let pos = win.outer_position().map_err(|e| e.to_string())?;
    let old = win.outer_size().map_err(|e| e.to_string())?;
    let d = (webcam_diameter(size) * scale).round() as u32;
    let y = pos.y + old.height as i32 - d as i32;
    win.set_size(PhysicalSize::new(d, d)).map_err(|e| e.to_string())?;
    win.set_position(PhysicalPosition::new(pos.x, y)).map_err(|e| e.to_string())?;
    Ok(())
}
