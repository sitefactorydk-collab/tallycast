//! Video engines: how the screen is grabbed and which encoder compresses it.
//!
//! On Windows the preferred path is Desktop Duplication (`ddagrab`) feeding a GPU
//! encoder (AMD AMF, NVIDIA NVENC or Intel Quick Sync). When the encoder accepts the
//! captured GPU textures directly ("direct"), frames never touch system memory. With
//! older FFmpeg builds (AMF before 8.0 rejects BGRA textures) frames are downloaded
//! first. Anything that fails falls back to GDI capture and x264 on the processor.
//!
//! What works is found by running FFmpeg briefly (`probe`), because an encoder being
//! compiled in says nothing about the graphics card actually present.

use crate::util::command;
use serde::{Deserialize, Serialize};
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

/// What this PC's FFmpeg can do. Cached on disk per FFmpeg binary and version.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Engines {
    pub ffmpeg_path: String,
    pub ffmpeg_version: String,
    /// Hardware encoders that encoded test frames successfully.
    pub encoders: Vec<Encoder>,
    /// Desktop Duplication works.
    pub ddagrab: bool,
    /// Hardware encoders that accept Desktop Duplication textures directly.
    pub direct: Vec<Encoder>,
}

/// The pipeline for one recording.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub capture: Capture,
    pub encoder: Encoder,
    /// GPU textures go straight into the encoder.
    pub direct: bool,
    /// `ddagrab` output index of the recorded monitor.
    pub output_idx: u32,
}

impl Plan {
    /// The path that works on every PC: GDI (or the native grabber) and x264.
    pub fn safe() -> Self {
        let capture = if cfg!(windows) { Capture::Gdigrab } else { Capture::Native };
        Plan { capture, encoder: Encoder::X264, direct: false, output_idx: 0 }
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
        let direct = if self.direct { ", direct from the GPU" } else { "" };
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
    let encoder = match Encoder::from_id(encoder_pref).filter(|enc| available(*enc)) {
        Some(enc) => enc,
        None => {
            // Automatic: an encoder that takes GPU frames directly beats one that doesn't.
            let direct_first = Encoder::HARDWARE.into_iter().find(|enc| dda && e.direct.contains(enc));
            direct_first
                .or_else(|| Encoder::HARDWARE.into_iter().find(|enc| e.encoders.contains(enc)))
                .unwrap_or(Encoder::X264)
        }
    };
    Plan {
        capture: if dda { Capture::Ddagrab } else { Capture::Gdigrab },
        encoder,
        direct: dda && e.direct.contains(&encoder),
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
    (plan.capture == Capture::Ddagrab && !plan.direct).then(|| "hwdownload,format=bgra".to_string())
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

/// Runs FFmpeg and reports whether it finished successfully within `timeout`.
fn runs_ok(ffmpeg: &Path, args: &[String], timeout: Duration) -> bool {
    let Ok(mut child) = command(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(40)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
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
        ffmpeg_path: ffmpeg.to_string_lossy().into_owned(),
        ffmpeg_version: version.to_string(),
        ..Engines::default()
    };
    if !cfg!(windows) {
        return e;
    }
    let timeout = Duration::from_secs(12);
    let synthetic = ["-f", "lavfi", "-i", "color=c=black:s=640x360:r=30"];
    let sw = |enc| Plan { capture: Capture::Gdigrab, encoder: enc, direct: false, output_idx: 0 };

    // The three encoders and Desktop Duplication are independent: test them together.
    let (encoders, ddagrab) = thread::scope(|scope| {
        let handles: Vec<_> = Encoder::HARDWARE
            .into_iter()
            .map(|enc| scope.spawn(move || runs_ok(ffmpeg, &test_args(&synthetic, &sw(enc), 10), timeout)))
            .collect();
        let dda_plan =
            Plan { capture: Capture::Ddagrab, encoder: Encoder::X264, direct: false, output_idx: 0 };
        let dda = runs_ok(ffmpeg, &test_args(&["-f", "lavfi", "-i", PROBE_DDA], &dda_plan, 3), timeout);
        let ok: Vec<Encoder> = Encoder::HARDWARE
            .into_iter()
            .zip(handles)
            .filter_map(|(enc, h)| h.join().unwrap_or(false).then_some(enc))
            .collect();
        (ok, dda)
    });
    e.encoders = encoders;
    e.ddagrab = ddagrab;

    if e.ddagrab {
        // Quick Sync would need its own device mapping; it always takes downloaded frames.
        for enc in e.encoders.iter().copied().filter(|enc| *enc != Encoder::Qsv) {
            let plan = Plan { capture: Capture::Ddagrab, encoder: enc, direct: true, output_idx: 0 };
            if runs_ok(ffmpeg, &test_args(&["-f", "lavfi", "-i", PROBE_DDA], &plan, 5), timeout) {
                e.direct.push(enc);
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
    fn encoder_ids_round_trip() {
        for e in [Encoder::Amf, Encoder::Nvenc, Encoder::Qsv, Encoder::X264] {
            assert_eq!(Encoder::from_id(e.id()), Some(e));
        }
        assert_eq!(Encoder::from_id("auto"), None);
    }

    #[test]
    fn args_for_each_encoder() {
        let amf = Plan { capture: Capture::Ddagrab, encoder: Encoder::Amf, direct: true, output_idx: 1 };
        let a = video_args(&amf, 23, 30).join(" ");
        assert_eq!(
            a,
            "-c:v h264_amf -usage transcoding -quality balanced -rc cqp -qp_i 22 -qp_p 22 -bf 0 -g 60"
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
