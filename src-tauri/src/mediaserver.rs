//! A tiny HTTP server on 127.0.0.1 that streams recordings to the editor's `<video>`.
//!
//! The web view needs a URL with byte-range support to play and seek a local MP4.
//! Only files explicitly registered get a (random) URL; nothing else is reachable.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub struct MediaServer {
    port: u16,
    files: Arc<Mutex<HashMap<String, PathBuf>>>,
}

impl MediaServer {
    pub fn start() -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let files: Arc<Mutex<HashMap<String, PathBuf>>> = Arc::default();
        let shared = files.clone();
        thread::Builder::new().name("media-server".into()).spawn(move || {
            for stream in listener.incoming().flatten() {
                let files = shared.clone();
                let _ = thread::Builder::new().name("media-conn".into()).spawn(move || serve(stream, &files));
            }
        })?;
        Ok(Self { port, files })
    }

    /// Returns a URL that serves `path` (the same URL each time for the same file).
    pub fn url_for(&self, path: &Path) -> String {
        let mut files = self.files.lock().unwrap_or_else(|p| p.into_inner());
        let token = match files.iter().find(|(_, p)| p.as_path() == path) {
            Some((t, _)) => t.clone(),
            None => {
                let t = random_token();
                files.insert(t.clone(), path.to_path_buf());
                t
            }
        };
        let ext =
            path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "mp4".into());
        format!("http://127.0.0.1:{}/m/{token}.{ext}", self.port)
    }
}

fn random_token() -> String {
    let mut out = String::new();
    for i in 0..2u64 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(i ^ crate::util::now_ms() as u64);
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

struct Request {
    method: String,
    path: String,
    range: Option<String>,
    keep_alive: bool,
}

fn read_request(reader: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let version = parts.next().unwrap_or("HTTP/1.1").to_string();
    let mut range = None;
    let mut keep_alive = version != "HTTP/1.0";
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).ok()? == 0 {
            return None;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            if k == "range" {
                range = Some(v.to_string());
            } else if k == "connection" {
                keep_alive = !v.eq_ignore_ascii_case("close");
            }
        }
    }
    Some(Request { method, path, range, keep_alive })
}

/// Parses `bytes=a-b`, `bytes=a-` or `bytes=-n` into an inclusive range within `len`.
pub fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    let spec = header.trim().strip_prefix("bytes=")?;
    let spec = spec.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    if len == 0 {
        return None;
    }
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        if n == 0 {
            return None;
        }
        (len.saturating_sub(n), len - 1)
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() { len - 1 } else { b.parse::<u64>().ok()?.min(len - 1) };
        (start, end)
    };
    if start > end || start >= len {
        return None;
    }
    Some((start, end))
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("mp4") | Some("m4v") => "video/mp4",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        _ => "application/octet-stream",
    }
}

fn serve(stream: TcpStream, files: &Mutex<HashMap<String, PathBuf>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let Ok(mut writer) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    while let Some(req) = read_request(&mut reader) {
        let ok = respond(&mut writer, &req, files).is_ok();
        if !ok || !req.keep_alive {
            break;
        }
    }
}

fn simple(w: &mut TcpStream, status: &str) -> io::Result<()> {
    write!(w, "HTTP/1.1 {status}\r\nContent-Length: 0\r\nAccess-Control-Allow-Origin: *\r\n\r\n")
}

fn respond(w: &mut TcpStream, req: &Request, files: &Mutex<HashMap<String, PathBuf>>) -> io::Result<()> {
    if req.method != "GET" && req.method != "HEAD" {
        return simple(w, "405 Method Not Allowed");
    }
    let token = req.path.strip_prefix("/m/").and_then(|rest| rest.split(['.', '?']).next()).unwrap_or("");
    let path = files.lock().unwrap_or_else(|p| p.into_inner()).get(token).cloned();
    let Some(path) = path else { return simple(w, "404 Not Found") };
    let Ok(mut file) = File::open(&path) else { return simple(w, "404 Not Found") };
    let len = file.metadata()?.len();

    let (status, start, end) = match req.range.as_deref() {
        Some(r) => match parse_range(r, len) {
            Some((a, b)) => ("206 Partial Content", a, b),
            None => {
                return write!(
                    w,
                    "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{len}\r\nContent-Length: 0\r\n\r\n"
                );
            }
        },
        None => ("200 OK", 0, len.saturating_sub(1)),
    };
    let count = if len == 0 { 0 } else { end - start + 1 };
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {}\r\nContent-Length: {count}\r\nAccept-Ranges: bytes\r\n\
         Access-Control-Allow-Origin: *\r\nCache-Control: no-cache\r\n",
        content_type(&path)
    );
    if status.starts_with("206") {
        head.push_str(&format!("Content-Range: bytes {start}-{end}/{len}\r\n"));
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes())?;
    if req.method == "HEAD" || count == 0 {
        return w.flush();
    }
    file.seek(SeekFrom::Start(start))?;
    // The player aborts the connection whenever it seeks; that ends up here as an error.
    io::copy(&mut file.take(count), w)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(parse_range("bytes=90-500", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=5-2", 100), None);
        assert_eq!(parse_range("items=0-1", 100), None);
    }

    #[test]
    fn serves_registered_files_with_ranges() {
        let dir = std::env::temp_dir().join(format!("tallycast-ms-{}", crate::util::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("clip.mp4");
        std::fs::write(&file, (0u8..=255).collect::<Vec<u8>>()).unwrap();
        let server = MediaServer::start().unwrap();
        let url = server.url_for(&file);
        assert_eq!(url, server.url_for(&file));
        let path = url.split_once(&format!(":{}", server.port)).unwrap().1.to_string();

        let mut s = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nRange: bytes=10-13\r\n\r\n").unwrap();
        let mut reader = BufReader::new(s.try_clone().unwrap());
        let mut head = String::new();
        loop {
            let mut l = String::new();
            reader.read_line(&mut l).unwrap();
            if l == "\r\n" {
                break;
            }
            head.push_str(&l);
        }
        assert!(head.starts_with("HTTP/1.1 206"), "{head}");
        assert!(head.contains("Content-Range: bytes 10-13/256"));
        let mut body = [0u8; 4];
        reader.read_exact(&mut body).unwrap();
        assert_eq!(body, [10, 11, 12, 13]);

        // Same connection, unknown token.
        write!(s, "GET /m/nope.mp4 HTTP/1.1\r\nConnection: close\r\n\r\n").unwrap();
        let mut rest = String::new();
        reader.read_to_string(&mut rest).unwrap();
        assert!(rest.starts_with("HTTP/1.1 404"), "{rest}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
