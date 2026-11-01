//! A recording session.
//!
//! Each run between start/resume and pause/stop is a *segment*: one FFmpeg process
//! writing a Matroska file (crash-tolerant), fed by fresh audio feeds. Pausing simply
//! ends the current segment; stopping concatenates all segments losslessly into MP4.

use crate::audio::{self, AudioFeed, AudioKind};
use crate::ffmpeg;
use crate::wins::MonitorInfo;
use serde::Serialize;
use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// A capture rectangle in physical pixels, absolute desktop coordinates.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl CaptureRect {
    pub fn from_monitor(m: &MonitorInfo) -> Self {
        Self { x: m.x, y: m.y, width: m.width, height: m.height }.even()
    }

    /// Builds a rect from coordinates relative to `m`, clamped to the monitor.
    pub fn within_monitor(m: &MonitorInfo, x: i32, y: i32, w: u32, h: u32) -> Self {
        let x = x.clamp(0, m.width as i32 - 2);
        let y = y.clamp(0, m.height as i32 - 2);
        let w = w.min(m.width - x as u32).max(2);
        let h = h.min(m.height - y as u32).max(2);
        Self { x: m.x + x, y: m.y + y, width: w, height: h }.even()
    }

    /// H.264 with yuv420p needs even dimensions.
    pub fn even(self) -> Self {
        Self { width: (self.width & !1).max(2), height: (self.height & !1).max(2), ..self }
    }
}

pub struct RecordConfig {
    pub ffmpeg: PathBuf,
    pub rect: CaptureRect,
    pub monitor: MonitorInfo,
    pub fps: u32,
    pub crf: u32,
    pub show_cursor: bool,
    pub mic_enabled: bool,
    pub mic_device: Option<String>,
    pub system_audio: bool,
    pub temp_dir: PathBuf,
}

type Log = Arc<Mutex<VecDeque<String>>>;

struct Segment {
    child: Child,
    stdin: Option<ChildStdin>,
    feeds: Vec<AudioFeed>,
    path: PathBuf,
    started: Instant,
    log: Log,
}

impl Drop for Segment {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.feeds.clear();
    }
}

fn log_tail(log: &Log) -> String {
    let lines: Vec<String> = log
        .lock()
        .map(|l| l.iter().rev().take(6).cloned().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .rev()
        .collect();
    if lines.is_empty() {
        String::new()
    } else {
        format!("\n\nFFmpeg said:\n{}", lines.join("\n"))
    }
}

pub struct Session {
    cfg: RecordConfig,
    done: Vec<PathBuf>,
    current: Option<Segment>,
    accumulated: Duration,
    counter: u32,
}

impl Session {
    pub fn start(cfg: RecordConfig) -> Result<Self, String> {
        fs::create_dir_all(&cfg.temp_dir).map_err(|e| format!("Could not create a temp folder ({e})."))?;
        let mut session =
            Self { cfg, done: Vec::new(), current: None, accumulated: Duration::ZERO, counter: 0 };
        if let Err(e) = session.start_segment() {
            let _ = fs::remove_dir_all(&session.cfg.temp_dir);
            return Err(e);
        }
        Ok(session)
    }

    pub fn rect(&self) -> CaptureRect {
        self.cfg.rect
    }

    pub fn is_paused(&self) -> bool {
        self.current.is_none()
    }

    pub fn elapsed(&self) -> Duration {
        self.accumulated + self.current.as_ref().map(|s| s.started.elapsed()).unwrap_or_default()
    }

    pub fn pause(&mut self) -> Result<(), String> {
        if self.current.is_some() {
            self.stop_segment()
        } else {
            Ok(())
        }
    }

    pub fn resume(&mut self) -> Result<(), String> {
        if self.current.is_none() {
            self.start_segment()
        } else {
            Ok(())
        }
    }

    /// Stops recording and writes the final MP4. Returns the duration in seconds.
    pub fn finish(mut self, out: &Path) -> Result<f64, String> {
        if let Err(e) = self.stop_segment() {
            if self.done.is_empty() {
                let _ = fs::remove_dir_all(&self.cfg.temp_dir);
                return Err(e);
            }
            eprintln!("[tallycast] last segment failed, keeping earlier ones: {e}");
        }
        if self.done.is_empty() {
            let _ = fs::remove_dir_all(&self.cfg.temp_dir);
            return Err("Nothing was recorded.".into());
        }
        if let Some(dir) = out.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("Could not create the output folder ({e})."))?;
        }
        ffmpeg::concat(&self.cfg.ffmpeg, &self.done, &self.cfg.temp_dir, out)?;
        let _ = fs::remove_dir_all(&self.cfg.temp_dir);
        Ok(self.accumulated.as_secs_f64())
    }

    /// Stops immediately and throws everything away.
    pub fn discard(mut self) {
        self.current.take(); // Segment::drop kills FFmpeg
        let _ = fs::remove_dir_all(&self.cfg.temp_dir);
    }

    fn start_segment(&mut self) -> Result<(), String> {
        let path = self.cfg.temp_dir.join(format!("part-{:03}.mkv", self.counter));
        self.counter += 1;

        let mut feeds = Vec::new();
        if self.cfg.mic_enabled {
            let feed = audio::start_feed(AudioKind::Microphone(self.cfg.mic_device.clone()))
                .map_err(|e| format!("Microphone: {e}"))?;
            feeds.push(feed);
        }
        if self.cfg.system_audio && audio::loopback_supported() {
            let feed =
                audio::start_feed(AudioKind::SystemLoopback).map_err(|e| format!("System audio: {e}"))?;
            feeds.push(feed);
        }

        let args = ffmpeg::record_args(&self.cfg, &feeds, &path);
        eprintln!("[tallycast] ffmpeg {}", args.join(" "));
        let mut child = crate::util::command(&self.cfg.ffmpeg)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Could not start FFmpeg ({e})."))?;

        let log: Log = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(stderr) = child.stderr.take() {
            let log = log.clone();
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    // Benign: large screen frames exceed the probe buffer, but the frame
                    // rate is set explicitly with -framerate.
                    if line.contains("not enough frames to estimate rate") {
                        continue;
                    }
                    eprintln!("[ffmpeg] {line}");
                    if let Ok(mut l) = log.lock() {
                        l.push_back(line);
                        if l.len() > 40 {
                            l.pop_front();
                        }
                    }
                }
            });
        }
        let stdin = child.stdin.take();
        let mut seg = Segment { child, stdin, feeds, path, started: Instant::now(), log };

        // Catch immediate failures (bad device, unsupported option, …).
        let check = Instant::now();
        while check.elapsed() < Duration::from_millis(900) {
            if let Ok(Some(_)) = seg.child.try_wait() {
                thread::sleep(Duration::from_millis(150));
                return Err(format!("FFmpeg stopped right after starting.{}", log_tail(&seg.log)));
            }
            thread::sleep(Duration::from_millis(50));
        }
        seg.started = check; // close enough to when frames began flowing
        self.current = Some(seg);
        Ok(())
    }

    fn stop_segment(&mut self) -> Result<(), String> {
        let Some(mut seg) = self.current.take() else {
            return Ok(());
        };
        let recorded = seg.started.elapsed();

        // "q" on stdin asks FFmpeg to finish the file cleanly.
        if let Some(mut stdin) = seg.stdin.take() {
            let _ = stdin.write_all(b"q");
            let _ = stdin.flush();
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match seg.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(40)),
                _ => {
                    let _ = seg.child.kill();
                    let _ = seg.child.wait();
                    break;
                }
            }
        }
        seg.feeds.clear();
        self.accumulated += recorded;

        let size = fs::metadata(&seg.path).map(|m| m.len()).unwrap_or(0);
        if size > 1024 {
            self.done.push(seg.path.clone());
            Ok(())
        } else {
            Err(format!("FFmpeg did not write any video.{}", log_tail(&seg.log)))
        }
    }
}
