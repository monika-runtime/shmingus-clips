//! A tiny read-only HTTP server for clip playback.
//!
//! WebKitGTK hands `<video>` sources to GStreamer's HTTP stack, which knows
//! nothing about Tauri's `asset://`/`http://asset.localhost` protocol handler
//! (images go through WebKit itself, media does not). So clips are served from
//! a loopback HTTP server with `Range` support instead.
//!
//! Only clips that are in the library index are reachable: the URL carries the
//! clip id, never a filesystem path.

use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::error::{AppError, Result};
use crate::library::Library;

const MIME: &str = "video/mp4";

pub struct MediaServer {
    base: String,
}

impl MediaServer {
    pub fn start(library: Arc<Library>) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|err| AppError::msg(format!("could not start the media server: {err}")))?;
        let address = listener
            .local_addr()
            .map_err(|err| AppError::msg(format!("media server address: {err}")))?;

        thread::Builder::new()
            .name("clipforge-media".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(stream) => {
                            let library = library.clone();
                            thread::spawn(move || {
                                if let Err(err) = handle(stream, &library) {
                                    log::debug!("media request failed: {err}");
                                }
                            });
                        }
                        Err(err) => log::warn!("media server accept failed: {err}"),
                    }
                }
            })
            .map_err(|err| AppError::msg(format!("could not spawn the media server: {err}")))?;

        let base = format!("http://{address}");
        log::info!("media server listening on {base}");
        Ok(Self { base })
    }

    /// Used when the port could not be bound: the app still works, only video
    /// playback is unavailable.
    pub fn unavailable() -> Self {
        Self {
            base: String::new(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }
}

fn handle(mut stream: TcpStream, library: &Library) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut raw = [0u8; 8192];
    let read = stream.read(&mut raw)?;
    let request = String::from_utf8_lossy(&raw[..read]);
    let mut lines = request.lines();
    let Some(request_line) = lines.next() else {
        return write_empty(&mut stream, 400, "Bad Request");
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();

    if method != "GET" && method != "HEAD" {
        return write_empty(&mut stream, 405, "Method Not Allowed");
    }

    let id = target
        .strip_prefix("/media/")
        .and_then(|rest| rest.split(['?', '#']).next())
        .filter(|id| !id.is_empty());
    let Some(id) = id else {
        return write_empty(&mut stream, 404, "Not Found");
    };
    let Some(path) = library.find_path(id) else {
        return write_empty(&mut stream, 404, "Not Found");
    };
    let Ok(metadata) = std::fs::metadata(&path) else {
        return write_empty(&mut stream, 404, "Not Found");
    };
    if !metadata.is_file() {
        return write_empty(&mut stream, 404, "Not Found");
    }
    let len = metadata.len();

    let range_header = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("range")
            .then(|| value.trim())
    });

    let Some(range) = range_header else {
        write_headers(&mut stream, 200, "OK", len, None)?;
        if method == "GET" {
            stream_file(&mut stream, &path, 0, len)?;
        }
        return Ok(());
    };

    let Some((start, end)) = parse_range(range, len) else {
        let headers = format!(
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{len}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        return stream.write_all(headers.as_bytes());
    };

    let count = end + 1 - start;
    write_headers(
        &mut stream,
        206,
        "Partial Content",
        count,
        Some((start, end, len)),
    )?;
    if method == "GET" {
        stream_file(&mut stream, &path, start, count)?;
    }
    Ok(())
}

fn write_empty(stream: &mut TcpStream, status: u16, reason: &str) -> std::io::Result<()> {
    let headers =
        format!("HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(headers.as_bytes())
}

fn write_headers(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    length: u64,
    range: Option<(u64, u64, u64)>,
) -> std::io::Result<()> {
    let mut headers = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {MIME}\r\n\
         Content-Length: {length}\r\n\
         Accept-Ranges: bytes\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n"
    );
    if let Some((start, end, total)) = range {
        headers.push_str(&format!("Content-Range: bytes {start}-{end}/{total}\r\n"));
    }
    headers.push_str("\r\n");
    stream.write_all(headers.as_bytes())
}

fn stream_file(
    stream: &mut TcpStream,
    path: &std::path::Path,
    start: u64,
    count: u64,
) -> std::io::Result<()> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut remaining = count;
    let mut buffer = vec![0u8; 64 * 1024];
    while remaining > 0 {
        let want = remaining.min(buffer.len() as u64) as usize;
        let read = file.read(&mut buffer[..want])?;
        if read == 0 {
            break;
        }
        stream.write_all(&buffer[..read])?;
        remaining -= read as u64;
    }
    Ok(())
}

/// Parse a single-range `Range` header (`bytes=start-end`, `bytes=start-`,
/// `bytes=-suffix`). Multi-range requests fall back to the first range, which is
/// all a media player needs.
fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    if len == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?;
    let spec = spec.split(',').next()?.trim();
    let (start_raw, end_raw) = spec.split_once('-')?;

    let start: Option<u64> = if start_raw.is_empty() {
        None
    } else {
        Some(start_raw.parse().ok()?)
    };
    let end: Option<u64> = if end_raw.is_empty() {
        None
    } else {
        Some(end_raw.parse().ok()?)
    };

    match (start, end) {
        (Some(start), Some(end)) => {
            (start <= end && start < len).then(|| (start, end.min(len - 1)))
        }
        (Some(start), None) => (start < len).then(|| (start, len - 1)),
        // A suffix range keeps the last N bytes.
        (None, Some(suffix)) => (suffix > 0).then(|| (len.saturating_sub(suffix), len - 1)),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn parses_range_forms() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=0-99999", 1000), Some((0, 999)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("bytes=5-2", 1000), None);
        assert_eq!(parse_range("bytes=-0", 1000), None);
        assert_eq!(parse_range("nonsense", 1000), None);
        assert_eq!(parse_range("bytes=0-10", 0), None);
    }
}
