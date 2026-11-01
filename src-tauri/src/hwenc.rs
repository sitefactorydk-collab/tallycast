//! Video engines: how the screen is grabbed and which encoder compresses it.
//!
//! On Windows the preferred path is Desktop Duplication (`ddagrab`) feeding a GPU
//! encoder (AMD AMF, NVIDIA NVENC or Intel Quick Sync). When the encoder accepts the
//! captured GPU textures directly ("direct"), frames never touch system memory. Some
//! drivers only take NV12 textures; then FFmpeg 8's `scale_d3d11` converts on the GPU
//! first ("converted"). Otherwise frames are downloaded to system memory first. Anything that fails falls back to GDI capture and x264 on the processor.
//!
//! What works is found by running FFmpeg briefly (`probe`), because an encoder being
//! compiled in says nothing about the graphics card actually present.

use crate::util::command;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Encoder {
    Amf,
    Nvenc,
    Qsv,
    X264,
}

impl Encoder {
    pub const HARDWARE: [Encoder; 3] = [Encoder::Amf, Encoder::Nvenc, Encoder::Qsv];

    pub fn id(self) -> &'static str {
        match self {
            Encoder::Amf => "amf",
            Encoder::Nvenc => "nvenc",
            Encoder::Qsv => "qsv",
            Encoder::X264 => "x264",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [Encoder::Amf, Encoder::Nvenc, Encoder::Qsv, Encoder::X264].into_iter().find(|e| e.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Encoder::Amf => "AMD graphics (AMF)",
            Encoder::Nvenc => "NVIDIA graphics (NVENC)",
            Encoder::Qsv => "Intel graphics (Quick Sync)",
            Encoder::X264 => "Processor (x264)",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Capture {
    /// Desktop Duplication (Windows 8+), GPU based.
    Ddagrab,
    /// GDI BitBlt (Windows), CPU based, works everywhere.
    Gdigrab,
    /// The platform grabber on macOS / Linux.
    Native,
}

/// Bumped whenever `probe` tests something new, so cached results are redone.
pub const SCHEMA: u32 = 2;

/// What this PC's FFmpeg can do. Cached on disk per FFmpeg binary and version.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Engines {
    pub schema: u32,
    pub ffmpeg_path: String,
    pub ffmpeg_version: String,
    /// Hardware encoders that encoded test frames successfully.
    pub encoders: Vec<Encoder>,
    /// Desktop Duplication works.
    pub ddagrab: bool,
    /// Hardware encoders that accept Desktop Duplication textures directly.
    pub direct: Vec<Encoder>,
    /// Hardware encoders that take them after a GPU conversion to NV12.
    pub converted: Vec<Encoder>,
    /// Why tests failed (FFmpeg's last error lines), for the log and diagnostics.
    pub notes: Vec<String>,
}

/// The pipeline for one recording.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub capture: Capture,
    pub encoder: Encoder,
    /// Frames stay on the GPU from capture to encoder.
    pub direct: bool,
    /// ...after `scale_d3d11` converts them to NV12 (only with `direct`).
    pub convert: bool,
    /// `ddagrab` output index of the recorded monitor.
    pub output_idx: u32,
}

impl Plan {
    /// The path that works on every PC: GDI (or the native grabber) and x264.
    pub fn safe() -> Self {
        let capture = if cfg!(windows) { Capture::Gdigrab } else { Capture::Native };
        Plan { capture, encoder: Encoder::X264, direct: false, convert: false, output_idx: 0 }
    }

    pub fn is_safe(&self) -> bool {
        *self == Plan::safe()
    }

    pub fn describe(&self) -> String {
        let capture = match self.capture {
            Capture::Ddagrab => "Desktop Duplication",
            Capture::Gdigrab => "GDI capture",
            Capture::Native => "screen capture",
        };
        let direct = match (self.direct, self.convert) {
            (true, false) => ", direct from the GPU",
            (true, true) => ", converted and encoded on the GPU",
            _ => "",
        };
        format!("{capture} + {}{direct}", self.encoder.label())
    }
}

/// Picks the pipeline. `encoder_pref` is "auto" or an encoder id; `capture_pref` is
/// "auto", "ddagrab" or "gdigrab". `output_idx` is the monitor's Desktop Duplication
/// output, if it has one.
pub fn choose(e: &Engines, encoder_pref: &str, capture_pref: &str, output_idx: Option<u32>) -> Plan {
    if !cfg!(windows) {
        return Plan::safe();
    }
    choose_windows(e, encoder_pref, capture_pref, output_idx)
}

fn choose_windows(e: &Engines, encoder_pref: &str, capture_pref: &str, output_idx: Option<u32>) -> Plan {
    let dda = e.ddagrab && capture_pref != "gdigrab" && output_idx.is_some();
    let available = |enc: Encoder| enc == Encoder::X264 || e.encoders.contains(&enc);
    let on_gpu = |enc: &Encoder| e.direct.contains(enc) || e.converted.contains(enc);
    let encoder = match Encoder::from_id(encoder_pref).filter(|enc| available(*enc)) {
        Some(enc) => enc,
        None => {
            // Automatic: an encoder that takes GPU frames directly beats one that doesn't.
            let direct_first = Encoder::HARDWARE.into_iter().find(|enc| dda && on_gpu(enc));
            direct_first
                .or_else(|| Encoder::HARDWARE.into_iter().find(|enc| e.encoders.contains(enc)))
                .unwrap_or(Encoder::X264)
        }
    };
    Plan {
        capture: if dda { Capture::Ddagrab } else { Capture::Gdigrab },
        encoder,
        direct: dda && on_gpu(&encoder),
        convert: dda && !e.direct.contains(&encoder) && e.converted.contains(&encoder),
        output_idx: output_idx.unwrap_or(0),
    }
}

fn s<T: ToString>(v: T) -> String {
    v.to_string()
}

/// The `ddagrab` source for a region relative to the monitor's top-left corner.
pub fn ddagrab_source(output_idx: u32, fps: u32, cursor: bool, x: i32, y: i32, w: u32, h: u32) -> String {
    format!(
        "ddagrab=output_idx={output_idx}:framerate={fps}:draw_mouse={}:offset_x={x}:offset_y={y}:video_size={w}x{h}",
        u8::from(cursor)
    )
}

/// Filter to apply to the captured video before encoding, if any.
pub fn capture_filter(plan: &Plan) -> Option<String> {
    match (plan.capture, plan.direct, plan.convert) {
        (Capture::Ddagrab, false, _) => Some("hwdownload,format=bgra".into()),
        (Capture::Ddagrab, true, true) => Some("scale_d3d11=format=nv12".into()),
        _ => None,
    }
}

/// Encoder arguments. `crf` is the x264 quality (lower = better); hardware encoders
/// get a constant quantizer one step better, since they compress less efficiently.
/// B-frames are off and keyframes come every two seconds, so the editor's smart cut
/// can copy whole groups of pictures.
pub fn video_args(plan: &Plan, crf: u32, fps: u32) -> Vec<String> {
    let gop = s(fps.max(1) * 2);
    let q = s(crf.saturating_sub(1).clamp(10, 40));
    let mut a: Vec<String> = match plan.encoder {
        Encoder::X264 => {
            return vec![
                s("-c:v"),
                s("libx264"),
                s("-preset"),
                s("veryfast"),
                s("-crf"),
                s(crf),
                s("-pix_fmt"),
                s("yuv420p"),
                s("-g"),
                gop,
            ]
        }
        Encoder::Amf => vec![
            s("-c:v"),
            s("h264_amf"),
            s("-usage"),
            s("transcoding"),
            s("-quality"),
            s("balanced"),
            s("-rc"),
            s("cqp"),
            s("-qp_i"),
            q.clone(),
            s("-qp_p"),
            q,
            // VBAQ only works with bitrate modes; say so instead of letting AMF warn.
            s("-vbaq"),
            s("0"),
        ],
        Encoder::Nvenc => {
            vec![s("-c:v"), s("h264_nvenc"), s("-preset"), s("p4"), s("-rc"), s("constqp"), s("-qp"), q]
        }
        Encoder::Qsv => vec![
            s("-c:v"),
            s("h264_qsv"),
            s("-preset"),
            s("medium"),
            s("-global_quality"),
            q,
            s("-look_ahead"),
            s("0"),
        ],
    };
    a.extend([s("-bf"), s("0"), s("-g"), gop]);
    if !plan.direct {
        a.extend([s("-pix_fmt"), s("nv12")]);
    }
    a
}

// ───────────────────────────── probing ─────────────────────────────

/// Runs FFmpeg; `Err` carries the reason (FFmpeg's last error lines, or a timeout).
fn run(ffmpeg: &Path, args: &[String], timeout: Duration) -> Result<(), String> {
    let mut child = command(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stderr = child.stderr.take();
    let reader = thread::spawn(move || {
        let mut text = String::new();
        if let Some(err) = stderr.as_mut() {
            let _ = err.read_to_string(&mut text);
        }
        text
    });
    let start = Instant::now();
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break if status.success() { Ok(()) } else { Err(String::new()) },
            Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(40)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("timed out".to_string());
            }
        }
    };
    let text = reader.join().unwrap_or_default();
    outcome.map_err(|e| if e.is_empty() { last_lines(&text) } else { e })
}

/// The last two non-empty lines, joined: usually the actual reason.
fn last_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let tail = &lines[lines.len().saturating_sub(2)..];
    if tail.is_empty() {
        "failed".into()
    } else {
        tail.join(" / ")
    }
}

const PROBE_DDA: &str = "ddagrab=output_idx=0:framerate=30:video_size=320x240";

fn test_args(input: &[&str], plan: &Plan, frames: u32) -> Vec<String> {
    let mut a: Vec<String> = input.iter().map(|x| x.to_string()).collect();
    if let Some(f) = capture_filter(plan) {
        a.extend([s("-vf"), f]);
    }
    a.extend([s("-frames:v"), s(frames)]);
    a.extend(video_args(plan, 23, 30));
    a.extend([s("-f"), s("null"), s("-")]);
    a
}

/// Finds out what works on this PC. Takes a few seconds; Windows only.
pub fn probe(ffmpeg: &Path, version: &str) -> Engines {
    let mut e = Engines {
        schema: SCHEMA,
        ffmpeg_path: ffmpeg.to_string_lossy().into_owned(),
        ffmpeg_version: version.to_string(),
        ..Engines::default()
    };
    if !cfg!(windows) {
        return e;
    }
    let timeout = Duration::from_secs(12);
    let synthetic = ["-f", "lavfi", "-i", "color=c=black:s=640x360:r=30"];
    let dda_input = ["-f", "lavfi", "-i", PROBE_DDA];
    let plan = |capture, encoder, direct, convert| Plan { capture, encoder, direct, convert, output_idx: 0 };

    // The three encoders and Desktop Duplication are independent: test them together.
    let (results, dda) = thread::scope(|scope| {
        let handles: Vec<_> = Encoder::HARDWARE
            .into_iter()
            .map(|enc| {
                let p = plan(Capture::Gdigrab, enc, false, false);
                scope.spawn(move || run(ffmpeg, &test_args(&synthetic, &p, 10), timeout))
            })
            .collect();
        let dda_plan = plan(Capture::Ddagrab, Encoder::X264, false, false);
        let dda = run(ffmpeg, &test_args(&dda_input, &dda_plan, 3), timeout);
        let results: Vec<(Encoder, Result<(), String>)> = Encoder::HARDWARE
            .into_iter()
            .zip(handles)
            .map(|(enc, h)| (enc, h.join().unwrap_or_else(|_| Err("panicked".into()))))
            .collect();
        (results, dda)
    });
    for (enc, r) in results {
        match r {
            Ok(()) => e.encoders.push(enc),
            Err(why) => e.notes.push(format!("{}: {why}", enc.id())),
        }
    }
    match dda {
        Ok(()) => e.ddagrab = true,
        Err(why) => e.notes.push(format!("ddagrab: {why}")),
    }

    if e.ddagrab {
        // Quick Sync would need its own device mapping; it always takes downloaded frames.
        for enc in e.encoders.clone().into_iter().filter(|enc| *enc != Encoder::Qsv) {
            let direct = plan(Capture::Ddagrab, enc, true, false);
            match run(ffmpeg, &test_args(&dda_input, &direct, 5), timeout) {
                Ok(()) => e.direct.push(enc),
                Err(why) => {
                    e.notes.push(format!("{} direct: {why}", enc.id()));
                    let converted = plan(Capture::Ddagrab, enc, true, true);
                    match run(ffmpeg, &test_args(&dda_input, &converted, 5), timeout) {
                        Ok(()) => e.converted.push(enc),
                        Err(why) => e.notes.push(format!("{} converted: {why}", enc.id())),
                    }
                }
            }
        }
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engines(enc: &[Encoder], dda: bool, direct: &[Encoder]) -> Engines {
        Engines { encoders: enc.to_vec(), ddagrab: dda, direct: direct.to_vec(), ..Default::default() }
    }

    #[test]
    fn converted_path_uses_scale_d3d11() {
        use Encoder::*;
        let e = Engines { encoders: vec![Amf], ddagrab: true, converted: vec![Amf], ..Default::default() };
        let p = choose_windows(&e, "auto", "auto", Some(0));
        assert_eq!((p.capture, p.encoder, p.direct, p.convert), (Capture::Ddagrab, Amf, true, true));
        assert_eq!(capture_filter(&p).as_deref(), Some("scale_d3d11=format=nv12"));
        assert!(!video_args(&p, 23, 30).contains(&"-pix_fmt".to_string()));
        assert!(p.describe().ends_with("converted and encoded on the GPU"));
        // Raw textures win over conversion when both work.
        let both = Engines { direct: vec![Amf], ..e };
        let p = choose_windows(&both, "auto", "auto", Some(0));
        assert_eq!((p.direct, p.convert), (true, false));
    }

    #[test]
    fn last_lines_picks_the_reason() {
        let text = "a\n\nFormat bgra is not supported\nError initializing\n";
        assert_eq!(last_lines(text), "Format bgra is not supported / Error initializing");
        assert_eq!(last_lines(""), "failed");
    }

    #[test]
    fn encoder_ids_round_trip() {
        for e in [Encoder::Amf, Encoder::Nvenc, Encoder::Qsv, Encoder::X264] {
            assert_eq!(Encoder::from_id(e.id()), Some(e));
        }
        assert_eq!(Encoder::from_id("auto"), None);
    }

    #[test]
    fn args_for_each_encoder() {
        let amf = Plan {
            capture: Capture::Ddagrab,
            encoder: Encoder::Amf,
            direct: true,
            convert: false,
            output_idx: 1,
        };
        let a = video_args(&amf, 23, 30).join(" ");
        assert_eq!(
            a,
            "-c:v h264_amf -usage transcoding -quality balanced -rc cqp -qp_i 22 -qp_p 22 -vbaq 0 -bf 0 -g 60"
        );
        assert_eq!(capture_filter(&amf), None);
        let amf_dl = Plan { direct: false, ..amf };
        assert!(video_args(&amf_dl, 23, 30).join(" ").ends_with("-g 60 -pix_fmt nv12"));
        assert_eq!(capture_filter(&amf_dl).as_deref(), Some("hwdownload,format=bgra"));
        let x = video_args(&Plan::safe(), 18, 60).join(" ");
        assert_eq!(x, "-c:v libx264 -preset veryfast -crf 18 -pix_fmt yuv420p -g 120");
        assert_eq!(
            ddagrab_source(1, 60, true, 866, 142, 2834, 1754),
            "ddagrab=output_idx=1:framerate=60:draw_mouse=1:offset_x=866:offset_y=142:video_size=2834x1754"
        );
    }

    #[test]
    fn choose_prefers_direct_gpu_paths() {
        use Encoder::*;
        let choose = choose_windows;
        // AMD with FFmpeg 8: Desktop Duplication straight into AMF.
        let e = engines(&[Amf], true, &[Amf]);
        let p = choose(&e, "auto", "auto", Some(0));
        assert_eq!((p.capture, p.encoder, p.direct), (Capture::Ddagrab, Amf, true));
        // AMD with FFmpeg 7: still AMF, frames downloaded.
        let p = choose(&engines(&[Amf], true, &[]), "auto", "auto", Some(2));
        assert_eq!((p.capture, p.encoder, p.direct, p.output_idx), (Capture::Ddagrab, Amf, false, 2));
        // Monitor not on the default adapter: GDI + AMF.
        let p = choose(&e, "auto", "auto", None);
        assert_eq!((p.capture, p.encoder, p.direct), (Capture::Gdigrab, Amf, false));
        // Asked for the processor.
        assert_eq!(choose(&e, "x264", "auto", Some(0)).encoder, X264);
        // Asked for NVENC on an AMD PC: falls back to automatic.
        assert_eq!(choose(&e, "nvenc", "auto", Some(0)).encoder, Amf);
        // Forced GDI.
        assert_eq!(choose(&e, "auto", "gdigrab", Some(0)).capture, Capture::Gdigrab);
        // Nothing works: the safe path.
        let p = choose(&engines(&[], false, &[]), "auto", "auto", Some(0));
        assert_eq!((p.capture, p.encoder, p.direct), (Capture::Gdigrab, X264, false));
        // Intel only, with Desktop Duplication: Quick Sync on downloaded frames.
        let p = choose(&engines(&[Qsv], true, &[]), "auto", "auto", Some(0));
        assert_eq!((p.capture, p.encoder, p.direct), (Capture::Ddagrab, Qsv, false));
        // NVIDIA laptop with Intel graphics too: the direct-capable encoder wins.
        assert_eq!(choose(&engines(&[Nvenc, Qsv], true, &[Nvenc]), "auto", "auto", Some(0)).encoder, Nvenc);
    }

    #[cfg(not(windows))]
    #[test]
    fn other_platforms_use_the_safe_path() {
        let e = engines(&[Encoder::Amf], true, &[Encoder::Amf]);
        assert!(choose(&e, "auto", "auto", Some(0)).is_safe());
    }
}
