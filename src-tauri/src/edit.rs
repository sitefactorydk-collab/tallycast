//! Trim-and-cut edits and the export that applies them.
//!
//! An edit is an in point, an out point and any number of cut-out spans. What is left
//! ("keep spans") is written to a new MP4; the original is never touched.
//!
//! Export has two strategies:
//! * **Smart** (default): whole GOPs inside a keep span are stream-copied, untouched.
//!   Only the frames between a cut and the next keyframe (and from the last keyframe to
//!   the end of a span) are re-encoded. Pieces are joined as MPEG-TS, which carries the
//!   H.264 parameter sets in-band, so copied and re-encoded pieces can follow each other.
//!   Sound is always re-encoded as one continuous track, so it stays sample-accurate.
//! * **Re-encode**: one FFmpeg pass with trim/concat filters. Slower, always exact,
//!   and the fallback when the source is not H.264 or cannot be indexed.

use crate::media::{MediaInfo, VideoIndex};
use crate::util::{command, path_str};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};

/// Returned as the error text when the user cancels an export.
pub const CANCELLED: &str = "cancelled";
const NOTHING_LEFT: &str = "Nothing is left to export: every part of the video is cut.";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

impl Span {
    pub fn len(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Edits {
    pub in_point: f64,
    /// `None` means the end of the video.
    pub out_point: Option<f64>,
    pub cuts: Vec<Span>,
}

/// The parts of `[0, duration]` that survive the edit, in order. Spans shorter than
/// 10 ms are dropped.
pub fn keep_spans(duration: f64, e: &Edits) -> Vec<Span> {
    let clamp = |t: f64| t.clamp(0.0, duration);
    let a = clamp(e.in_point);
    let b = clamp(e.out_point.unwrap_or(duration));
    if b <= a {
        return Vec::new();
    }
    let mut cuts: Vec<Span> = e
        .cuts
        .iter()
        .map(|c| Span { start: clamp(c.start.min(c.end)), end: clamp(c.start.max(c.end)) })
        .filter(|c| c.len() > 0.0)
        .collect();
    cuts.sort_by(|x, y| x.start.partial_cmp(&y.start).unwrap_or(std::cmp::Ordering::Equal));

    let mut keep = Vec::new();
    let mut cursor = a;
    for c in cuts {
        if c.end <= cursor {
            continue;
        }
        if c.start >= b {
            break;
        }
        if c.start > cursor {
            keep.push(Span { start: cursor, end: c.start });
        }
        cursor = cursor.max(c.end);
    }
    if cursor < b {
        keep.push(Span { start: cursor, end: b });
    }
    keep.retain(|s| s.len() >= 0.01);
    keep
}

pub fn total(spans: &[Span]) -> f64 {
    spans.iter().map(Span::len).sum()
}

// ───────────────────────────── smart plan ─────────────────────────────

/// A run of frames `[first, end)` (indices into `VideoIndex::pts`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Piece {
    Copy { first: usize, end: usize },
    Encode { first: usize, end: usize },
}

/// Frame ranges for each keep span: frames whose start time lies in `[start, end)`.
pub fn frame_spans(spans: &[Span], idx: &VideoIndex, duration: f64) -> Vec<(usize, usize)> {
    spans
        .iter()
        .map(|s| {
            let first = idx.frame_at_or_after(s.start);
            let end = if s.end >= duration - idx.frame_dur * 0.5 {
                idx.len()
            } else {
                idx.frame_at_or_after(s.end)
            };
            (first, end.max(first))
        })
        .filter(|(f, e)| e > f)
        .collect()
}

/// Splits each frame range into copied whole GOPs and re-encoded edges.
///
/// Copying `[k1, k2)` is only safe when both ends are keyframes (or `k2` is the end of
/// the file): the recorder's x264 output uses closed GOPs, so the packets between two
/// keyframes in decode order are exactly the frames between them in display order.
pub fn plan(ranges: &[(usize, usize)], idx: &VideoIndex) -> Vec<Piece> {
    let n = idx.len();
    let is_boundary = |j: usize| j == n || idx.key[j];
    let mut out = Vec::new();
    for &(f, e) in ranges {
        let k1 = (f..e).find(|&i| idx.key[i]);
        let k2 = k1.and_then(|k1| ((k1 + 1)..=e).rev().find(|&j| is_boundary(j)));
        match (k1, k2) {
            (Some(k1), Some(k2)) => {
                if f < k1 {
                    out.push(Piece::Encode { first: f, end: k1 });
                }
                out.push(Piece::Copy { first: k1, end: k2 });
                if k2 < e {
                    out.push(Piece::Encode { first: k2, end: e });
                }
            }
            _ => out.push(Piece::Encode { first: f, end: e }),
        }
    }
    out
}

// ───────────────────────────── running FFmpeg ─────────────────────────────

pub struct Runner<'a> {
    pub ffmpeg: &'a Path,
    pub cancel: &'a AtomicBool,
    pub progress: &'a mut dyn FnMut(f64),
    /// Seconds of output already accounted for, and the total, for the progress bar.
    done: f64,
    total: f64,
}

impl<'a> Runner<'a> {
    pub fn new(
        ffmpeg: &'a Path,
        cancel: &'a AtomicBool,
        progress: &'a mut dyn FnMut(f64),
        total: f64,
    ) -> Self {
        Self { ffmpeg, cancel, progress, done: 0.0, total: total.max(1e-6) }
    }

    fn report(&mut self, extra: f64) {
        let f = ((self.done + extra) / self.total).clamp(0.0, 1.0);
        (self.progress)(f);
    }

    /// Runs FFmpeg with `args`; `weight` is how much of the total this step represents
    /// and `out_secs` how many seconds of output it writes (to scale its progress).
    fn run(&mut self, args: &[String], weight: f64, out_secs: f64) -> Result<(), String> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.into());
        }
        let mut full: Vec<String> =
            ["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-progress", "pipe:1", "-nostats"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        full.extend_from_slice(args);
        let mut child = command(self.ffmpeg)
            .args(&full)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Could not run FFmpeg ({e})."))?;

        let mut stderr = child.stderr.take();
        let err_reader = std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(e) = stderr.as_mut() {
                let _ = e.read_to_string(&mut s);
            }
            s
        });

        let mut cancelled = false;
        if let Some(stdout) = child.stdout.take() {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if self.cancel.load(Ordering::SeqCst) {
                    let _ = child.kill();
                    cancelled = true;
                    break;
                }
                if let Some(us) = line.strip_prefix("out_time_us=") {
                    if let Ok(us) = us.trim().parse::<f64>() {
                        let frac = if out_secs > 0.0 { (us / 1e6 / out_secs).clamp(0.0, 1.0) } else { 0.0 };
                        self.report(weight * frac);
                    }
                }
            }
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        let err = err_reader.join().unwrap_or_default();
        if cancelled || self.cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.into());
        }
        if !status.success() {
            let msg = err.trim();
            return Err(if msg.is_empty() { "FFmpeg stopped unexpectedly.".into() } else { msg.to_string() });
        }
        self.done += weight;
        self.report(0.0);
        Ok(())
    }
}

fn s<T: ToString>(v: T) -> String {
    v.to_string()
}

fn secs(t: f64) -> String {
    format!("{:.6}", t.max(0.0))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    /// Every frame stream-copied.
    Copy,
    /// Copied GOPs with re-encoded edges.
    Smart,
    /// Everything re-encoded.
    Reencode,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub method: Method,
    pub duration: f64,
    /// Seconds of video that had to be re-encoded.
    pub reencoded: f64,
}

pub struct ExportJob<'a> {
    pub src: &'a Path,
    pub out: &'a Path,
    pub work_dir: &'a Path,
    pub info: &'a MediaInfo,
    pub spans: &'a [Span],
    pub crf: u32,
}

const AUDIO_ARGS: [&str; 6] = ["-c:a", "aac", "-b:a", "192k", "-ar", "48000"];

fn x264_args(crf: u32) -> Vec<String> {
    vec![s("-c:v"), s("libx264"), s("-preset"), s("veryfast"), s("-crf"), s(crf), s("-pix_fmt"), s("yuv420p")]
}

/// One FFmpeg pass that trims, joins and re-encodes everything.
///
/// `half_frame` is non-zero when `spans` are snapped to frame start times: the video
/// trim then cuts half a frame early so each boundary frame lands on the right side,
/// while sound is cut at the exact frame times.
pub fn export_reencode(
    job: &ExportJob,
    spans: &[Span],
    half_frame: f64,
    runner: &mut Runner,
) -> Result<ExportReport, String> {
    if spans.is_empty() {
        return Err(NOTHING_LEFT.into());
    }
    // Seek close to the first kept frame so a late in point doesn't decode from 0.
    let offset = (spans[0].start - half_frame).max(0.0);
    let until = spans[spans.len() - 1].end - offset;
    let n = spans.len();
    let audio = job.info.has_audio;

    let mut g = String::new();
    if n > 1 {
        g.push_str(&format!("[0:v:0]split={n}"));
        for i in 0..n {
            g.push_str(&format!("[sv{i}]"));
        }
        g.push(';');
        if audio {
            g.push_str(&format!("[0:a:0]asplit={n}"));
            for i in 0..n {
                g.push_str(&format!("[sa{i}]"));
            }
            g.push(';');
        }
    }
    let mut joins = String::new();
    for (i, sp) in spans.iter().enumerate() {
        let (a, b) = (sp.start - offset, sp.end - offset);
        let (va, vb) = (a - half_frame, b - half_frame);
        let vin = if n > 1 { format!("[sv{i}]") } else { "[0:v:0]".into() };
        g.push_str(&format!("{vin}trim=start={}:end={},setpts=PTS-STARTPTS[v{i}];", secs(va), secs(vb)));
        joins.push_str(&format!("[v{i}]"));
        if audio {
            let ain = if n > 1 { format!("[sa{i}]") } else { "[0:a:0]".into() };
            g.push_str(&format!("{ain}atrim=start={}:end={},asetpts=PTS-STARTPTS[a{i}];", secs(a), secs(b)));
            joins.push_str(&format!("[a{i}]"));
        }
    }
    g.push_str(&format!("{joins}concat=n={n}:v=1:a={}[v]", u8::from(audio)));
    if audio {
        g.push_str("[a]");
    }

    let mut args = vec![s("-ss"), secs(offset), s("-t"), secs(until + 0.5), s("-i"), path_str(job.src)];
    args.extend([s("-filter_complex"), g, s("-map"), s("[v]")]);
    if audio {
        args.extend([s("-map"), s("[a]")]);
    }
    args.extend(x264_args(job.crf));
    if audio {
        args.extend(AUDIO_ARGS.iter().map(|a| a.to_string()));
    }
    args.extend([s("-movflags"), s("+faststart"), path_str(job.out)]);

    let dur = total(spans);
    runner.run(&args, runner.total, dur)?;
    Ok(ExportReport { method: Method::Reencode, duration: dur, reencoded: dur })
}

/// Copies whole GOPs and re-encodes only the edges around each cut.
pub fn export_smart(
    job: &ExportJob,
    idx: &VideoIndex,
    ranges: &[(usize, usize)],
    runner: &mut Runner,
) -> Result<ExportReport, String> {
    if ranges.is_empty() {
        return Err(NOTHING_LEFT.into());
    }
    let pieces = plan(ranges, idx);
    let fd = idx.frame_dur;
    let src = path_str(job.src);
    // Durations come from the real timestamps, which need not be evenly spaced.
    let dur = |p: &Piece| match *p {
        Piece::Copy { first, end } | Piece::Encode { first, end } => idx.time_of(end) - idx.time_of(first),
    };
    let out_dur: f64 = pieces.iter().map(dur).sum();
    let encoded: f64 = pieces.iter().filter(|p| matches!(p, Piece::Encode { .. })).map(dur).sum();

    // Progress weights: encoding dominates, copying is quick, the final mux re-encodes sound.
    let copy_w = 0.04;
    let mux_w = if job.info.has_audio { 0.08 * out_dur } else { 0.01 * out_dur };
    let weights: Vec<f64> = pieces
        .iter()
        .map(|p| match p {
            Piece::Encode { .. } => dur(p),
            Piece::Copy { .. } => dur(p) * copy_w,
        })
        .collect();
    let scale = runner.total / (weights.iter().sum::<f64>() + mux_w).max(1e-6);

    let mut list = String::new();
    for (i, piece) in pieces.iter().enumerate() {
        let file = job.work_dir.join(format!("piece{i:04}.ts"));
        let mut args: Vec<String> = Vec::new();
        match *piece {
            Piece::Copy { first, end } => {
                // Land exactly on the keyframe: seeking picks the keyframe at or before.
                args.extend([s("-ss"), secs(idx.pts[first] + fd * 0.25), s("-i"), src.clone()]);
                args.extend([
                    s("-map"),
                    s("0:v:0"),
                    s("-c:v"),
                    s("copy"),
                    s("-bsf:v"),
                    s("h264_mp4toannexb"),
                ]);
                args.extend([s("-frames:v"), s(end - first)]);
            }
            Piece::Encode { first, end } => {
                // Accurate seek: decodes from the previous keyframe, drops frames before `first`.
                args.extend([s("-ss"), secs(idx.pts[first] - fd * 0.5), s("-i"), src.clone()]);
                args.extend([s("-map"), s("0:v:0")]);
                args.extend(x264_args(job.crf));
                args.extend([s("-frames:v"), s(end - first)]);
            }
        }
        args.extend([s("-an"), s("-f"), s("mpegts"), path_str(&file)]);
        runner.run(&args, weights[i] * scale, dur(piece))?;

        let name = path_str(&file).replace('\\', "/").replace('\'', "'\\''");
        list.push_str(&format!("file '{name}'\nduration {}\n", secs(dur(piece))));
    }
    let list_file = job.work_dir.join("pieces.txt");
    fs::write(&list_file, list).map_err(|e| e.to_string())?;

    // Join the pieces, and cut the sound to the same frame-exact spans.
    let mut args = vec![s("-f"), s("concat"), s("-safe"), s("0"), s("-i"), path_str(&list_file)];
    if job.info.has_audio {
        args.extend([s("-i"), src.clone()]);
        let mut g = String::new();
        let n = ranges.len();
        if n > 1 {
            g.push_str(&format!("[1:a:0]asplit={n}"));
            for i in 0..n {
                g.push_str(&format!("[sa{i}]"));
            }
            g.push(';');
        }
        let mut joins = String::new();
        for (i, &(f, e)) in ranges.iter().enumerate() {
            let (a, b) = (idx.time_of(f), idx.time_of(e));
            let ain = if n > 1 { format!("[sa{i}]") } else { "[1:a:0]".into() };
            g.push_str(&format!("{ain}atrim=start={}:end={},asetpts=PTS-STARTPTS[a{i}];", secs(a), secs(b)));
            joins.push_str(&format!("[a{i}]"));
        }
        g.push_str(&format!("{joins}concat=n={n}:v=0:a=1[a]"));
        args.extend([s("-filter_complex"), g, s("-map"), s("0:v"), s("-map"), s("[a]")]);
        args.extend(AUDIO_ARGS.iter().map(|a| a.to_string()));
    } else {
        args.extend([s("-map"), s("0:v")]);
    }
    args.extend([s("-c:v"), s("copy"), s("-movflags"), s("+faststart"), path_str(job.out)]);
    runner.run(&args, mux_w * scale, out_dur)?;

    let method = if encoded > 0.0 { Method::Smart } else { Method::Copy };
    Ok(ExportReport { method, duration: out_dur, reencoded: encoded })
}

/// Picks the strategy, runs it in a scratch folder and cleans up. On failure or
/// cancel the partial output file is removed.
pub fn export(
    job: &ExportJob,
    index: Option<&VideoIndex>,
    reencode: bool,
    runner: &mut Runner,
) -> Result<ExportReport, String> {
    fs::create_dir_all(job.work_dir).map_err(|e| e.to_string())?;
    let index = index.filter(|i| !i.is_empty());
    let result = match index {
        Some(idx) => {
            // Snap every span to whole frames, the same way the player shows them.
            let ranges = frame_spans(job.spans, idx, job.info.duration);
            let snapped: Vec<Span> =
                ranges.iter().map(|&(f, e)| Span { start: idx.time_of(f), end: idx.time_of(e) }).collect();
            let half = idx.frame_dur * 0.5;
            if !reencode && job.info.video_codec == "h264" {
                export_smart(job, idx, &ranges, runner).or_else(|e| {
                    if e == CANCELLED || e == NOTHING_LEFT {
                        return Err(e);
                    }
                    eprintln!("[tallycast] smart export failed, re-encoding instead: {e}");
                    runner.done = 0.0;
                    export_reencode(job, &snapped, half, runner)
                })
            } else {
                export_reencode(job, &snapped, half, runner)
            }
        }
        None => export_reencode(job, job.spans, 0.0, runner),
    };
    let _ = fs::remove_dir_all(job.work_dir);
    if result.is_err() {
        let _ = fs::remove_file(job.out);
    }
    result
}

/// Scratch folder for one export.
pub fn work_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("tallycast-export-{tag}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edits(i: f64, o: Option<f64>, cuts: &[(f64, f64)]) -> Edits {
        Edits {
            in_point: i,
            out_point: o,
            cuts: cuts.iter().map(|&(start, end)| Span { start, end }).collect(),
        }
    }

    #[test]
    fn keep_spans_basic() {
        assert_eq!(keep_spans(10.0, &Edits::default()), vec![Span { start: 0.0, end: 10.0 }]);
        let k = keep_spans(10.0, &edits(1.0, Some(9.0), &[(3.0, 4.0), (6.0, 7.0)]));
        assert_eq!(
            k,
            vec![Span { start: 1.0, end: 3.0 }, Span { start: 4.0, end: 6.0 }, Span { start: 7.0, end: 9.0 }]
        );
        assert!((total(&k) - 6.0).abs() < 1e-9);
    }

    #[test]
    fn keep_spans_overlapping_reversed_and_outside() {
        let k = keep_spans(10.0, &edits(2.0, Some(8.0), &[(5.0, 3.0), (4.0, 6.0), (0.0, 2.5), (7.5, 20.0)]));
        assert_eq!(k, vec![Span { start: 2.5, end: 3.0 }, Span { start: 6.0, end: 7.5 }]);
        assert!(keep_spans(10.0, &edits(5.0, Some(5.0), &[])).is_empty());
        assert!(keep_spans(10.0, &edits(0.0, None, &[(0.0, 10.0)])).is_empty());
    }

    /// 30 fps, keyframe every 10 frames, 50 frames.
    fn index() -> VideoIndex {
        let n = 50;
        VideoIndex {
            pts: (0..n).map(|i| i as f64 / 30.0).collect(),
            key: (0..n).map(|i| i % 10 == 0).collect(),
            frame_dur: 1.0 / 30.0,
        }
    }

    #[test]
    fn frame_spans_snap_to_frames() {
        let idx = index();
        let r = frame_spans(
            &[Span { start: 0.1, end: 0.5 }, Span { start: 1.0, end: 50.0 / 30.0 }],
            &idx,
            50.0 / 30.0,
        );
        assert_eq!(r, vec![(3, 15), (30, 50)]);
    }

    #[test]
    fn plan_copies_whole_gops_and_encodes_edges() {
        let idx = index();
        assert_eq!(
            plan(&[(3, 25)], &idx),
            vec![
                Piece::Encode { first: 3, end: 10 },
                Piece::Copy { first: 10, end: 20 },
                Piece::Encode { first: 20, end: 25 }
            ]
        );
        // Starts on a keyframe and runs to the end of the file: pure copy.
        assert_eq!(plan(&[(20, 50)], &idx), vec![Piece::Copy { first: 20, end: 50 }]);
        // Ends exactly on a keyframe boundary.
        assert_eq!(plan(&[(0, 30)], &idx), vec![Piece::Copy { first: 0, end: 30 }]);
        // No full GOP inside: encode it all.
        assert_eq!(plan(&[(12, 18)], &idx), vec![Piece::Encode { first: 12, end: 18 }]);
        assert_eq!(plan(&[(5, 15)], &idx), vec![Piece::Encode { first: 5, end: 15 }]);
    }
}
