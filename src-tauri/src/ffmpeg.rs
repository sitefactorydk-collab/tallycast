//! Locating FFmpeg and building its command lines.

use crate::audio::AudioFeed;
use crate::recorder::RecordConfig;
use crate::settings::Settings;
use crate::util::{command, path_str};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

#[cfg(windows)]
const EXE: &str = "ffmpeg.exe";
#[cfg(not(windows))]
const EXE: &str = "ffmpeg";

/// Finds an FFmpeg binary: explicit setting → next to the app → PATH → common install spots.
pub fn find(settings: &Settings) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(p) = settings.ffmpeg_path.as_ref().filter(|p| !p.trim().is_empty()) {
        candidates.push(PathBuf::from(p.trim()));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(EXE));
            candidates.push(dir.join("ffmpeg").join(EXE));
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            candidates.push(dir.join(EXE));
        }
    }

    #[cfg(windows)]
    {
        // winget installs here; the PATH change only reaches apps started afterwards.
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let winget = PathBuf::from(local).join("Microsoft").join("WinGet");
            candidates.push(winget.join("Links").join(EXE));
            if let Ok(packages) = fs::read_dir(winget.join("Packages")) {
                for pkg in packages.flatten() {
                    if !pkg.file_name().to_string_lossy().to_lowercase().contains("ffmpeg") {
                        continue;
                    }
                    if let Ok(inner) = fs::read_dir(pkg.path()) {
                        for build in inner.flatten() {
                            candidates.push(build.path().join("bin").join(EXE));
                        }
                    }
                }
            }
        }
        for p in [
            r"C:\ffmpeg\bin\ffmpeg.exe",
            r"C:\Program Files\ffmpeg\bin\ffmpeg.exe",
            r"C:\ProgramData\chocolatey\bin\ffmpeg.exe",
        ] {
            candidates.push(PathBuf::from(p));
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            candidates.push(PathBuf::from(home).join("scoop").join("shims").join(EXE));
        }
    }
    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from("/opt/homebrew/bin/ffmpeg"));
        candidates.push(PathBuf::from("/usr/local/bin/ffmpeg"));
    }

    candidates.into_iter().find(|p| p.is_file())
}

/// First line of `ffmpeg -version`, e.g. "ffmpeg version 7.1-essentials_build".
pub fn version(ffmpeg: &Path) -> Option<String> {
    let out = command(ffmpeg).arg("-version").stdin(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next()?.trim();
    let short: String = line.split(" Copyright").next().unwrap_or(line).to_string();
    Some(short)
}

fn s<T: ToString>(v: T) -> String {
    v.to_string()
}

/// The screen-grab input for the current OS.
fn video_input(cfg: &RecordConfig) -> Vec<String> {
    let r = cfg.rect;
    let fps = s(cfg.fps);
    let cursor = if cfg.show_cursor { "1" } else { "0" };

    #[cfg(windows)]
    {
        vec![
            s("-f"),
            s("gdigrab"),
            s("-thread_queue_size"),
            s(1024),
            s("-framerate"),
            fps,
            s("-draw_mouse"),
            s(cursor),
            s("-offset_x"),
            s(r.x),
            s("-offset_y"),
            s(r.y),
            s("-video_size"),
            format!("{}x{}", r.width, r.height),
            s("-i"),
            s("desktop"),
        ]
    }
    #[cfg(target_os = "macos")]
    {
        let _ = r;
        vec![
            s("-f"),
            s("avfoundation"),
            s("-thread_queue_size"),
            s(1024),
            s("-framerate"),
            fps,
            s("-capture_cursor"),
            s(cursor),
            s("-pixel_format"),
            s("uyvy422"),
            s("-i"),
            format!("Capture screen {}:none", cfg.monitor.index),
        ]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
        vec![
            s("-f"),
            s("x11grab"),
            s("-thread_queue_size"),
            s(1024),
            s("-framerate"),
            fps,
            s("-draw_mouse"),
            s(cursor),
            s("-video_size"),
            format!("{}x{}", r.width, r.height),
            s("-i"),
            format!("{display}+{},{}", r.x, r.y),
        ]
    }
}

/// Video filter applied after grabbing (macOS grabs a whole screen and crops).
fn video_filter(cfg: &RecordConfig) -> Option<String> {
    if cfg!(target_os = "macos") {
        let r = cfg.rect;
        let m = &cfg.monitor;
        Some(format!("crop={}:{}:{}:{}", r.width, r.height, r.x - m.x, r.y - m.y))
    } else {
        None
    }
}

pub fn record_args(cfg: &RecordConfig, feeds: &[AudioFeed], out: &Path) -> Vec<String> {
    let mut a: Vec<String> = vec![s("-hide_banner"), s("-loglevel"), s("warning"), s("-nostats"), s("-y")];
    a.extend(video_input(cfg));

    for f in feeds {
        // The format is fully specified, so skip probing: otherwise FFmpeg spends
        // seconds analysing the live socket and the start of the recording is lost.
        a.extend([
            s("-thread_queue_size"),
            s(4096),
            s("-probesize"),
            s(32),
            s("-analyzeduration"),
            s(0),
            s("-f"),
            s("f32le"),
            s("-ar"),
            s(f.sample_rate),
            s("-ac"),
            s(f.channels),
            s("-i"),
            format!("tcp://127.0.0.1:{}", f.port),
        ]);
    }

    let vf = video_filter(cfg);
    match feeds.len() {
        0 => {
            if let Some(vf) = vf {
                a.extend([s("-vf"), vf]);
            }
            a.extend([s("-map"), s("0:v")]);
        }
        1 => {
            if let Some(vf) = vf {
                a.extend([s("-vf"), vf]);
            }
            a.extend([s("-map"), s("0:v"), s("-map"), s("1:a")]);
        }
        n => {
            // Mix every audio input into one stereo track.
            let mut graph = String::new();
            let mut labels = String::new();
            for i in 1..=n {
                graph.push_str(&format!("[{i}:a]aresample=48000,aformat=channel_layouts=stereo[a{i}];"));
                labels.push_str(&format!("[a{i}]"));
            }
            graph.push_str(&format!("{labels}amix=inputs={n}:duration=longest:normalize=0[aout]"));
            match vf {
                Some(vf) => {
                    graph = format!("[0:v]{vf}[vout];{graph}");
                    a.extend([s("-filter_complex"), graph, s("-map"), s("[vout]")]);
                }
                None => a.extend([s("-filter_complex"), graph, s("-map"), s("0:v")]),
            }
            a.extend([s("-map"), s("[aout]")]);
        }
    }

    a.extend([
        s("-c:v"),
        s("libx264"),
        s("-preset"),
        s("veryfast"),
        s("-crf"),
        s(cfg.crf),
        s("-pix_fmt"),
        s("yuv420p"),
        s("-g"),
        s(cfg.fps * 2),
    ]);
    if !feeds.is_empty() {
        a.extend([s("-c:a"), s("aac"), s("-b:a"), s("192k"), s("-ar"), s(48000)]);
    }
    a.push(path_str(out));
    a
}

/// Joins recorded segments into one MP4 without re-encoding.
pub fn concat(ffmpeg: &Path, parts: &[PathBuf], work_dir: &Path, out: &Path) -> Result<(), String> {
    let mut args: Vec<String> = vec![s("-hide_banner"), s("-loglevel"), s("error"), s("-y")];
    if parts.len() == 1 {
        args.extend([s("-i"), path_str(&parts[0])]);
    } else {
        let list = work_dir.join("parts.txt");
        let body: String = parts
            .iter()
            .map(|p| {
                // Forward slashes and escaped quotes keep the concat parser happy on Windows.
                let p = path_str(p).replace('\\', "/").replace('\'', "'\\''");
                format!("file '{p}'\n")
            })
            .collect();
        fs::write(&list, body).map_err(|e| e.to_string())?;
        args.extend([s("-f"), s("concat"), s("-safe"), s("0"), s("-i"), path_str(&list)]);
    }
    args.extend([s("-c"), s("copy"), s("-movflags"), s("+faststart"), path_str(out)]);

    let output = command(ffmpeg)
        .args(&args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Could not run FFmpeg ({e})."))?;
    if output.status.success() && out.exists() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(format!("Saving the video failed.\n\n{}", err.trim()))
    }
}

/// Writes a small JPEG preview of `video`. Returns false if FFmpeg could not.
pub fn thumbnail(ffmpeg: &Path, video: &Path, out: &Path) -> bool {
    let input = path_str(video);
    let output = path_str(out);
    for seek in ["0.5", "0"] {
        let args: [&str; 15] = [
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-ss",
            seek,
            "-i",
            input.as_str(),
            "-frames:v",
            "1",
            "-vf",
            "scale=480:-2",
            "-q:v",
            "4",
            output.as_str(),
        ];
        let ok = command(ffmpeg)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok && out.exists() {
            return true;
        }
    }
    false
}
