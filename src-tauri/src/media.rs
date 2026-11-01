//! Reading recordings for the editor: stream info, the frame/keyframe index,
//! waveform peaks and filmstrip frames. Everything goes through the `ffmpeg`
//! binary (the bundled build has no `ffprobe`).

use crate::util::{command, path_str};
use serde::Serialize;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub duration: f64,
    /// Container start time in seconds. Times everywhere else are relative to it.
    pub start: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// When the first video frame starts, relative to `start`. Frame `k` starts at
    /// about `video_start + k / fps`.
    pub video_start: f64,
    pub video_codec: String,
    pub has_audio: bool,
}

/// Reads the stream summary FFmpeg prints for the file, plus the first few video
/// packets to learn when the first frame starts.
pub fn probe(ffmpeg: &Path, src: &Path) -> Result<MediaInfo, String> {
    let out = command(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-copyts", "-i"])
        .arg(src)
        .args(["-map", "0:v:0", "-c", "copy", "-frames:v", "16", "-f", "framecrc", "-"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Could not run FFmpeg ({e})."))?;
    let text = String::from_utf8_lossy(&out.stderr);
    let mut info = parse_banner(&text).ok_or_else(|| {
        let last = text.lines().last().unwrap_or("").trim().to_string();
        format!("FFmpeg could not read this video. {last}")
    })?;
    let head = parse_framecrc(&String::from_utf8_lossy(&out.stdout), info.start);
    info.video_start = head.pts.first().copied().unwrap_or(0.0).max(0.0);
    Ok(info)
}

fn parse_clock(s: &str) -> Option<f64> {
    let mut parts = s.trim().split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let sec: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// Parses the "Input #0 …" block FFmpeg prints to stderr.
pub fn parse_banner(text: &str) -> Option<MediaInfo> {
    let mut duration = None;
    let mut start = 0.0;
    let mut video: Option<(String, u32, u32, f64)> = None;
    let mut has_audio = false;

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("Output #") {
            break;
        }
        if let Some(rest) = line.strip_prefix("Duration:") {
            let mut fields = rest.split(',');
            duration = fields.next().and_then(parse_clock);
            for f in fields {
                if let Some(v) = f.trim().strip_prefix("start:") {
                    start = v.trim().parse().unwrap_or(0.0);
                }
            }
        } else if line.starts_with("Stream #") {
            if let Some(idx) = line.find("Video: ") {
                if video.is_some() {
                    continue;
                }
                let desc = &line[idx + 7..];
                let codec: String =
                    desc.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                let mut width = 0;
                let mut height = 0;
                for tok in desc.split([' ', ',', '[', ']']) {
                    if let Some((w, h)) = tok.split_once('x') {
                        if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                            if w >= 16 && h >= 16 {
                                width = w;
                                height = h;
                                break;
                            }
                        }
                    }
                }
                // Prefer tbr: Matroska-joined recordings have millisecond timestamps, so
                // the average ("fps") comes out as 29.97 when the real rate is 30.
                let fps = number_before(desc, " tbr").or_else(|| number_before(desc, " fps")).unwrap_or(30.0);
                video = Some((codec, width, height, fps));
            } else if line.contains("Audio: ") {
                has_audio = true;
            }
        }
    }

    let (video_codec, width, height, fps) = video?;
    Some(MediaInfo {
        duration: duration.filter(|d| *d > 0.0)?,
        start,
        width,
        height,
        fps: if fps > 0.0 { fps } else { 30.0 },
        video_start: 0.0,
        video_codec,
        has_audio,
    })
}

/// In "…, 30 fps, 30 tbr" returns the number in front of `unit` ("90k" = 90000).
fn number_before(desc: &str, unit: &str) -> Option<f64> {
    let head = &desc[..desc.find(unit)?];
    let n = &head[head.rfind([',', ' ']).map(|i| i + 1).unwrap_or(0)..];
    match n.strip_suffix('k') {
        Some(k) => k.parse::<f64>().ok().map(|x| x * 1000.0),
        None => n.parse().ok(),
    }
}

// ───────────────────────────── frame index ─────────────────────────────

/// Every video frame's presentation time (seconds, relative to the container start),
/// in display order, and whether it is a keyframe.
#[derive(Clone, Debug, Default)]
pub struct VideoIndex {
    pub pts: Vec<f64>,
    pub key: Vec<bool>,
    /// Typical frame duration in seconds.
    pub frame_dur: f64,
}

impl VideoIndex {
    pub fn len(&self) -> usize {
        self.pts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pts.is_empty()
    }

    /// Index of the first frame whose display midpoint is at or after `t`. A span
    /// `[a, b)` keeps exactly the frames `frame_at_or_after(a)..frame_at_or_after(b)`,
    /// which is what the player shows between those two times.
    pub fn frame_at_or_after(&self, t: f64) -> usize {
        let eps = self.frame_dur * 0.5;
        self.pts.partition_point(|&p| p < t - eps)
    }

    /// When frame `i` starts showing; `len()` gives the end of the last frame.
    pub fn time_of(&self, i: usize) -> f64 {
        match self.pts.get(i) {
            Some(t) => *t,
            None => self.pts.last().map(|t| t + self.frame_dur).unwrap_or(0.0),
        }
    }
}

/// Lists every video packet without decoding (stream copy into the `framecrc` muxer).
pub fn index_video(ffmpeg: &Path, src: &Path, start: f64) -> Result<VideoIndex, String> {
    let out = command(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-copyts", "-i"])
        .arg(src)
        .args(["-map", "0:v:0", "-c", "copy", "-f", "framecrc", "-"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Could not run FFmpeg ({e})."))?;
    if !out.status.success() {
        return Err(format!("Could not index the video. {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let idx = parse_framecrc(&String::from_utf8_lossy(&out.stdout), start);
    if idx.is_empty() {
        return Err("The video has no frames FFmpeg could read.".into());
    }
    Ok(idx)
}

/// Parses `framecrc` output: `#tb 0: num/den` then
/// `stream, dts, pts, duration, size, crc[, F=0x…][, S=…]`. Packets without an `F=`
/// field carry exactly the keyframe flag.
pub fn parse_framecrc(text: &str, start: f64) -> VideoIndex {
    let mut tb = 1.0 / 90_000.0;
    let mut frames: Vec<(i64, bool)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#tb 0:") {
            if let Some((n, d)) = rest.trim().split_once('/') {
                if let (Ok(n), Ok(d)) = (n.trim().parse::<f64>(), d.trim().parse::<f64>()) {
                    if n > 0.0 && d > 0.0 {
                        tb = n / d;
                    }
                }
            }
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        if fields.len() < 6 || fields[0] != "0" {
            continue;
        }
        let Ok(pts) = fields[2].parse::<i64>() else { continue };
        let key = !fields[6..].iter().any(|f| f.starts_with("F="));
        frames.push((pts, key));
    }
    frames.sort_by_key(|f| f.0);
    frames.dedup_by_key(|f| f.0);

    let pts: Vec<f64> = frames.iter().map(|f| f.0 as f64 * tb - start).collect();
    let key: Vec<bool> = frames.iter().map(|f| f.1).collect();
    // The average, not the median: timestamps from the Matroska pause segments are
    // rounded to whole milliseconds, so frames alternate between 33 and 34 ms.
    let frame_dur = match (pts.first(), pts.last()) {
        (Some(a), Some(b)) if pts.len() > 1 && b > a => (b - a) / (pts.len() - 1) as f64,
        _ => 1.0 / 30.0,
    };
    VideoIndex { pts, key, frame_dur }
}

// ───────────────────────────── waveform ─────────────────────────────

pub const WAVE_RATE: u32 = 8000;

/// Peak level of the first audio track, `per_sec` values per second, 0–255.
pub fn waveform(ffmpeg: &Path, src: &Path, per_sec: u32) -> Result<Vec<u8>, String> {
    let mut child = command(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-i"])
        .arg(src)
        .args(["-map", "0:a:0", "-vn", "-ac", "1", "-ar"])
        .arg(WAVE_RATE.to_string())
        .args(["-f", "s16le", "-acodec", "pcm_s16le", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Could not run FFmpeg ({e})."))?;
    let mut stdout = child.stdout.take().ok_or("FFmpeg gave no output.")?;
    let peaks = peaks_from_pcm(&mut stdout, (WAVE_RATE / per_sec.max(1)).max(1) as usize);
    let _ = child.wait();
    Ok(peaks)
}

/// Reduces signed 16-bit mono PCM to one peak per `bucket` samples.
pub fn peaks_from_pcm(reader: &mut dyn Read, bucket: usize) -> Vec<u8> {
    let mut peaks = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut carry: Option<u8> = None;
    let mut count = 0usize;
    let mut peak = 0i32;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let mut i = 0;
        let mut sample = |lo: u8, hi: u8| {
            let v = (i16::from_le_bytes([lo, hi]) as i32).abs();
            peak = peak.max(v);
            count += 1;
            if count == bucket {
                peaks.push(((peak.min(32767) * 255) / 32767) as u8);
                count = 0;
                peak = 0;
            }
        };
        if let Some(lo) = carry.take() {
            sample(lo, buf[0]);
            i = 1;
        }
        while i + 1 < n {
            sample(buf[i], buf[i + 1]);
            i += 2;
        }
        if i < n {
            carry = Some(buf[i]);
        }
    }
    if count > 0 {
        peaks.push(((peak.min(32767) * 255) / 32767) as u8);
    }
    peaks
}

// ───────────────────────────── filmstrip ─────────────────────────────

/// One small JPEG of the frame at `t` seconds, `height` pixels tall.
pub fn frame_jpeg(ffmpeg: &Path, src: &Path, t: f64, height: u32) -> Option<Vec<u8>> {
    let input = path_str(src);
    for seek in [t, (t - 1.0).max(0.0)] {
        let out = command(ffmpeg)
            .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-ss"])
            .arg(format!("{seek:.3}"))
            .args(["-i", input.as_str(), "-frames:v", "1", "-vf"])
            .arg(format!("scale=-2:{height}"))
            .args(["-q:v", "5", "-f", "image2pipe", "-c:v", "mjpeg", "-"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if out.status.success() && out.stdout.len() > 4 && out.stdout[0] == 0xFF && out.stdout[1] == 0xD8 {
            return Some(out.stdout);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANNER: &str = r#"Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'Recording 2026-11-01 at 10.42.17.mp4':
  Metadata:
    major_brand     : isom
  Duration: 00:05:12.40, start: 0.000000, bitrate: 1234 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(progressive), 2860x1454 [SAR 1:1 DAR 1430:727], 1180 kb/s, 30 fps, 30 tbr, 15360 tbn (default)
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo, fltp, 192 kb/s (default)
At least one output file must be specified"#;

    #[test]
    fn banner_is_parsed() {
        let i = parse_banner(BANNER).unwrap();
        assert!((i.duration - 312.4).abs() < 1e-9);
        assert_eq!((i.width, i.height), (2860, 1454));
        assert_eq!(i.fps, 30.0);
        assert_eq!(i.video_codec, "h264");
        assert!(i.has_audio);
    }

    #[test]
    fn banner_without_audio_and_fractional_fps() {
        let text = "  Duration: 01:00:00.00, start: 0.021000, bitrate: 1 kb/s\n  Stream #0:0: Video: h264 (High), yuv420p, 1280x720, 29.97 fps, 29.97 tbr, 90k tbn";
        let i = parse_banner(text).unwrap();
        assert_eq!(i.duration, 3600.0);
        assert_eq!(i.start, 0.021);
        assert_eq!(i.fps, 29.97);
        assert!(!i.has_audio);
    }

    #[test]
    fn framecrc_is_parsed_and_sorted() {
        let text = "#tb 0: 1/15360\n0,      -1024,          0,      512,     4569, 0x64bf62b5\n0,       -512,       2048,      512,      100, 0x1, F=0x0\n0,          0,        512,      512,      100, 0x2, F=0x0\n0,        512,       1024,      512,      100, 0x3, F=0x0\n0,       1024,       1536,      512,      100, 0x4, F=0x0\n0,       1536,       2560,      512,     4000, 0x5\n";
        let idx = parse_framecrc(text, 0.0);
        assert_eq!(idx.len(), 6);
        assert_eq!(idx.key, vec![true, false, false, false, false, true]);
        assert!((idx.frame_dur - 1.0 / 30.0).abs() < 1e-9);
        assert!((idx.time_of(6) - 6.0 / 30.0).abs() < 1e-9);
        assert_eq!(idx.frame_at_or_after(0.0), 0);
        assert_eq!(idx.frame_at_or_after(0.034), 1);
        assert_eq!(idx.frame_at_or_after(0.05), 2);
    }

    #[test]
    fn pcm_peaks() {
        let samples: Vec<i16> = vec![0, 100, -32767, 5, 16384, 0, 0];
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let peaks = peaks_from_pcm(&mut bytes.as_slice(), 3);
        assert_eq!(peaks, vec![255, 127, 0]);
    }
}
