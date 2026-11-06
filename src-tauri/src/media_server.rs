//! Loopback HTTP server used by the in-app player.
//!
//! WebKitGTK cannot stream `<video>` from Tauri's custom `asset://` scheme (it reports a
//! `FormatError` although GStreamer decodes the same file fine over file:// and http://),
//! so clips are served over plain HTTP on 127.0.0.1 instead:
//!
//! * bound to the loopback interface only, on a random port;
//! * every URL contains a 128-bit random token generated at startup;
//! * only clips registered in the library are served, looked up **by id** — the request
//!   never contains a path, so there is no path traversal surface;
//! * `Range` requests (206 Partial Content) for instant seeking, `HEAD` support.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub struct MediaServer {
    pub port: u16,
    pub token: String,
}

impl MediaServer {
    pub fn clip_url(&self, id: i64) -> String {
        format!("http://127.0.0.1:{}/{}/clip/{id}", self.port, self.token)
    }
}

type Resolver = Arc<dyn Fn(i64) -> Option<PathBuf> + Send + Sync>;

fn random_token() -> String {
    let mut b = [0u8; 16];
    if getrandom::getrandom(&mut b).is_err() {
        // Extremely unlikely; fall back to time + pid entropy.
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        b = (t ^ (std::process::id() as u128) << 64).to_le_bytes();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn start(resolve: impl Fn(i64) -> Option<PathBuf> + Send + Sync + 'static) -> std::io::Result<MediaServer> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let token = random_token();
    let resolve: Resolver = Arc::new(resolve);
    let tok = token.clone();
    std::thread::Builder::new().name("vinci-media".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let (resolve, tok) = (resolve.clone(), tok.clone());
            let _ = std::thread::Builder::new().name("vinci-media-conn".into()).spawn(move || {
                let _ = handle(stream, &tok, &*resolve);
            });
        }
    })?;
    log::info!("media server on 127.0.0.1:{port}");
    Ok(MediaServer { port, token })
}

/// Parse `bytes=a-b`, `bytes=a-`, `bytes=-n` against a file of `size` bytes.
pub(crate) fn parse_range(h: &str, size: u64) -> Option<(u64, u64)> {
    let spec = h.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    if size == 0 {
        return None;
    }
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            (size.saturating_sub(n), size - 1)
        }
        (s, "") => (s.parse().ok()?, size - 1),
        (s, e) => (s.parse().ok()?, e.parse::<u64>().ok()?.min(size - 1)),
    };
    (start <= end && start < size).then_some((start, end))
}

fn content_type(p: &std::path::Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        _ => "video/mp4",
    }
}

fn handle(stream: TcpStream, token: &str, resolve: &dyn Fn(i64) -> Option<PathBuf>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let mut range = None;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.trim().eq_ignore_ascii_case("range") {
                range = Some(v.trim().to_string());
            }
        }
    }
    let mut out = stream;
    let deny = |out: &mut TcpStream, code: &str| {
        out.write_all(format!("HTTP/1.1 {code}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes())
    };
    if method != "GET" && method != "HEAD" {
        return deny(&mut out, "405 Method Not Allowed");
    }
    // /<token>/clip/<id>
    let mut segs = target.trim_start_matches('/').split('/');
    let id = match (segs.next(), segs.next(), segs.next(), segs.next()) {
        (Some(t), Some("clip"), Some(id), None) if t == token => id.parse::<i64>().ok(),
        _ => None,
    };
    let Some(path) = id.and_then(resolve) else { return deny(&mut out, "404 Not Found") };
    let Ok(mut file) = File::open(&path) else { return deny(&mut out, "404 Not Found") };
    let size = file.metadata()?.len();
    let (status, start, end) = match range.as_deref().map(|r| parse_range(r, size)) {
        Some(Some((s, e))) => ("206 Partial Content", s, e),
        Some(None) => {
            return out.write_all(
                format!(
                    "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{size}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            );
        }
        None => ("200 OK", 0, size.saturating_sub(1)),
    };
    let len = if size == 0 { 0 } else { end - start + 1 };
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {}\r\nContent-Length: {len}\r\nAccept-Ranges: bytes\r\nCache-Control: no-store\r\nConnection: close\r\n",
        content_type(&path)
    );
    if status.starts_with("206") {
        head.push_str(&format!("Content-Range: bytes {start}-{end}/{size}\r\n"));
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes())?;
    if method == "HEAD" || len == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::Start(start))?;
    let mut remaining = len;
    let mut buf = vec![0u8; 256 * 1024];
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        let n = file.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?; // a closed connection (seek) simply ends the thread
        remaining -= n as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=900-5000", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("bytes=5-1", 1000), None);
        assert_eq!(parse_range("items=0-1", 1000), None);
    }

    fn get(port: u16, path: &str, range: Option<&str>) -> (String, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let r = range.map(|r| format!("Range: {r}\r\n")).unwrap_or_default();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n{r}\r\n").as_bytes()).unwrap();
        let mut all = Vec::new();
        s.read_to_end(&mut all).unwrap();
        let pos = all.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (String::from_utf8_lossy(&all[..pos]).into_owned(), all[pos + 4..].to_vec())
    }

    #[test]
    fn serves_only_known_clips_with_token_and_ranges() {
        let dir = std::env::temp_dir().join(format!("vinci-media-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("clip.mp4");
        std::fs::write(&file, (0..=255u8).cycle().take(10_000).collect::<Vec<_>>()).unwrap();
        let f2 = file.clone();
        let srv = start(move |id| (id == 7).then(|| f2.clone())).unwrap();
        let base = format!("/{}/clip", srv.token);

        let (h, body) = get(srv.port, &format!("{base}/7"), None);
        assert!(h.starts_with("HTTP/1.1 200") && h.contains("Content-Type: video/mp4") && body.len() == 10_000, "{h}");
        let (h, body) = get(srv.port, &format!("{base}/7"), Some("bytes=100-199"));
        assert!(h.starts_with("HTTP/1.1 206") && h.contains("Content-Range: bytes 100-199/10000"), "{h}");
        assert_eq!(body, (100..200).map(|i| (i % 256) as u8).collect::<Vec<_>>());
        assert!(get(srv.port, &format!("{base}/8"), None).0.starts_with("HTTP/1.1 404"), "unknown id");
        assert!(get(srv.port, "/wrongtoken/clip/7", None).0.starts_with("HTTP/1.1 404"), "bad token");
        assert!(get(srv.port, &format!("{base}/../../etc/passwd"), None).0.starts_with("HTTP/1.1 404"), "no paths");
        assert!(get(srv.port, &format!("{base}/7"), Some("bytes=20000-")).0.starts_with("HTTP/1.1 416"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
