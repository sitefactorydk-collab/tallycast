//! Small helpers shared across modules.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// A `Command` that never flashes a console window on Windows.
pub fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Returns `path` if it does not exist, otherwise `name (2).ext`, `name (3).ext`, …
pub fn unique_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem =
        path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Recording".into());
    let ext = path.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    for n in 2..10_000 {
        let candidate = dir.join(format!("{stem} ({n}).{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path
}

/// Standard base64 (RFC 4648) encoder, used for thumbnail data URLs.
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Decodes standard base64 (RFC 4648, padding optional, whitespace ignored).
pub fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    fn value(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    // Accept a data URL too.
    let body = text.split_once(";base64,").map(|(_, b)| b).unwrap_or(text);
    let mut out = Vec::with_capacity(body.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &c in body.as_bytes() {
        if c == b'=' {
            break;
        }
        if c.is_ascii_whitespace() {
            continue;
        }
        let v = value(c).ok_or_else(|| format!("Invalid base64 character {:?}.", c as char))?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// Opens a file or folder with the system's default application.
pub fn open_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("{} no longer exists.", path_str(path)));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer.exe returns exit code 1 even on success, so the status is ignored.
        Command::new("explorer.exe")
            .raw_arg(format!("\"{}\"", path_str(path)))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn().map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(path).spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Shows a file selected in the system file manager.
pub fn reveal_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("{} no longer exists.", path_str(path)));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", path_str(path)))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg("-R").arg(path).spawn().map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent().map(Path::to_path_buf).unwrap_or_default()
        };
        Command::new("xdg-open").arg(dir).spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_round_trips() {
        for n in 0..64usize {
            let data: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(base64_decode(&base64(&data)).unwrap(), data);
        }
        assert_eq!(base64_decode("data:image/png;base64,Zm9v\nYmE=").unwrap(), b"fooba");
        assert!(base64_decode("Zm9v!").is_err());
    }
}
