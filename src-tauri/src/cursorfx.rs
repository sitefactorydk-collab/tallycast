//! Cursor halo and click effects (Windows).
//!
//! A small, see-through, click-through window follows the mouse pointer and draws the
//! halo and click ripples. It is *not* excluded from capture, so it is recorded with the
//! screen like the webcam bubble, and it works with both GDI and Desktop Duplication.
//! The pointer arrow itself is still drawn by FFmpeg, on top of the halo.
//!
//! A background thread polls the pointer and mouse buttons every few milliseconds,
//! moves the window (asynchronously, so a busy UI thread never holds it up) and tells
//! the page about presses and releases. It ends by itself once the window is closed.

use crate::settings::Settings;
use crate::wins::{self, MonitorInfo};
use tauri::AppHandle;

/// Halo diameter in logical pixels; the pointer scales with display scaling, so do we.
pub fn halo_diameter(size: &str) -> f64 {
    match size {
        "s" => 44.0,
        "l" => 80.0,
        _ => 60.0,
    }
}

/// The window is this many halo diameters wide, leaving room for the click ripple.
/// Must match `SPAN` in `ui/js/cursorfx.js`.
pub const SPAN: f64 = 2.2;

pub fn wanted(s: &Settings) -> bool {
    cfg!(windows) && (s.cursor_halo || s.click_effects)
}

/// "#RrGgBb" → "rrggbb" for the page URL; anything else falls back to `fallback`.
fn hex(color: &str, fallback: &str) -> String {
    let c = color.trim().trim_start_matches('#');
    if c.len() == 6 && c.chars().all(|ch| ch.is_ascii_hexdigit()) {
        c.to_ascii_lowercase()
    } else {
        fallback.to_string()
    }
}

/// Opens the effect window and starts following the pointer, if either effect is on.
pub fn start(app: &AppHandle, s: &Settings, mon: &MonitorInfo) -> Result<(), String> {
    if !wanted(s) {
        return Ok(());
    }
    let scale = if mon.scale > 0.0 { mon.scale } else { 1.0 };
    let size = (halo_diameter(&s.cursor_fx_size) * SPAN * scale).round() as u32;
    let url = format!(
        "cursor.html?halo={}&hc={}&click={}&cc={}",
        u8::from(s.cursor_halo),
        hex(&s.halo_color, "ffd60a"),
        u8::from(s.click_effects),
        hex(&s.click_color, "e23b3e"),
    );
    let win = wins::open_cursor(app, &url, size, scale)?;
    imp::follow(app.clone(), win)
}

#[cfg(windows)]
mod imp {
    use crate::wins;
    use std::mem::size_of;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::Duration;
    use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

    /// Each new effect window bumps this; an older follower thread then stops.
    static GENERATION: AtomicU64 = AtomicU64::new(0);

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct CursorInfo {
        cb_size: u32,
        flags: u32,
        cursor: isize,
        pos: Point,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetCursorInfo(info: *mut CursorInfo) -> i32;
        fn GetAsyncKeyState(key: i32) -> i16;
        fn GetSystemMetrics(index: i32) -> i32;
        fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32;
        fn SetWindowPos(hwnd: isize, after: isize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    }

    const CURSOR_SHOWING: u32 = 0x1;
    const SM_SWAPBUTTON: i32 = 23;
    const VK_LBUTTON: i32 = 0x01;
    const VK_RBUTTON: i32 = 0x02;
    const HWND_TOPMOST: isize = -1;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_NOOWNERZORDER: u32 = 0x0200;
    const SWP_ASYNCWINDOWPOS: u32 = 0x4000;

    /// How often the pointer is checked (~200 times a second; very cheap calls).
    const TICK: Duration = Duration::from_millis(5);

    pub fn follow(app: AppHandle, win: WebviewWindow) -> Result<(), String> {
        let hwnd = win.hwnd().map_err(|e| e.to_string())?.0 as isize;
        let gen = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        thread::Builder::new()
            .name("tallycast-cursorfx".into())
            .spawn(move || run(app, hwnd, gen))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn send(app: &AppHandle, what: &str) {
        let _ = app.emit_to(wins::CURSOR, "cursorfx", what.to_string());
    }

    fn run(app: AppHandle, hwnd: isize, gen: u64) {
        // GetAsyncKeyState reports physical buttons; follow the user's primary button.
        // SAFETY: plain Win32 queries with no pointers.
        let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
        let buttons = if swapped {
            [(VK_RBUTTON, "left"), (VK_LBUTTON, "right")]
        } else {
            [(VK_LBUTTON, "left"), (VK_RBUTTON, "right")]
        };
        let mut down = [false; 2];
        let mut last = (i32::MIN, i32::MIN);
        let mut shown = true;
        let mut tick: u32 = 0;

        loop {
            if GENERATION.load(Ordering::SeqCst) != gen {
                break;
            }
            // Closing the window (stop, cancel, error) ends this thread.
            if tick % 20 == 0 && app.get_webview_window(wins::CURSOR).is_none() {
                break;
            }

            let mut info = CursorInfo {
                cb_size: size_of::<CursorInfo>() as u32,
                flags: 0,
                cursor: 0,
                pos: Point { x: 0, y: 0 },
            };
            // SAFETY: GetCursorInfo only writes the struct it is given, sized via cb_size.
            if unsafe { GetCursorInfo(&mut info) } != 0 {
                // Apps may hide the pointer (typing, full-screen video): hide the halo too.
                let visible = info.flags & CURSOR_SHOWING != 0;
                if visible != shown {
                    shown = visible;
                    send(&app, if visible { "show" } else { "hide" });
                }
                let pos = (info.pos.x, info.pos.y);
                // Now and then, step back in front of windows opened later (the webcam).
                let restack = tick % 100 == 0;
                if pos != last || restack {
                    let mut r = Rect::default();
                    // SAFETY: GetWindowRect writes the RECT; SetWindowPos takes plain values.
                    // A stale handle (window just closed) makes both fail harmlessly.
                    unsafe {
                        if GetWindowRect(hwnd, &mut r) != 0 {
                            let (w, h) = (r.right - r.left, r.bottom - r.top);
                            let mut flags =
                                SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS;
                            if !restack {
                                flags |= SWP_NOZORDER;
                            }
                            SetWindowPos(hwnd, HWND_TOPMOST, pos.0 - w / 2, pos.1 - h / 2, 0, 0, flags);
                            last = pos;
                        }
                    }
                }
            }

            for (i, (vk, name)) in buttons.iter().enumerate() {
                // SAFETY: GetAsyncKeyState takes a virtual-key code and returns a state.
                let pressed = (unsafe { GetAsyncKeyState(*vk) } as u16 & 0x8000) != 0;
                if pressed != down[i] {
                    down[i] = pressed;
                    send(&app, &format!("{} {name}", if pressed { "down" } else { "up" }));
                }
            }

            tick = tick.wrapping_add(1);
            thread::sleep(TICK);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use tauri::{AppHandle, WebviewWindow};

    pub fn follow(_app: AppHandle, _win: WebviewWindow) -> Result<(), String> {
        Ok(())
    }
}
