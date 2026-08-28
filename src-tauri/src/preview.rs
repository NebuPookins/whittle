//! Streaming the file under edit to the webview's `<video>` element.
//!
//! The webview cannot load the file itself: Tauri's asset protocol and any
//! other custom URI scheme both route through `webkit_web_context_register_uri_scheme`,
//! and WebKitGTK's GStreamer media pipeline cannot source audio/video from a
//! custom scheme (WebKit bug #146351). A `file://` URL is blocked by the webview,
//! for the same reason a page cannot reach arbitrary local files. What GStreamer
//! *can* source natively is plain HTTP, so this runs a tiny loopback HTTP server
//! and hands the `<video>` element an `http://127.0.0.1:<port>/<token>` URL.
//!
//! Only the file currently open for preview is reachable, addressed by an
//! opaque token rather than by path: the webview cannot name a file, so it
//! cannot ask for one we did not offer. The server binds 127.0.0.1 only, so it
//! is not reachable from another machine.

use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Read, Seek, SeekFrom, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use crate::applog;

/// A file no larger than this is served whole (buffered) when no range was
/// requested. Larger files are streamed.
const INLINE_LIMIT: u64 = 16 * 1024 * 1024;

/// The file currently open for preview.
#[derive(Debug, Clone)]
struct Preview {
    token: u64,
    path: PathBuf,
    mime: &'static str,
}

#[derive(Default)]
struct PreviewState {
    current: RwLock<Option<Preview>>,
    tokens: AtomicU64,
}

impl PreviewState {
    /// Offer `path` for preview, replacing whatever was offered before, and
    /// return the token that addresses it.
    ///
    /// The token also cache-busts: a new file gets a new URL, so the webview
    /// cannot serve a previous file's bytes out of its own cache.
    fn offer(&self, path: PathBuf, mime: &'static str) -> u64 {
        let token = self.tokens.fetch_add(1, Ordering::SeqCst) + 1;
        let mut slot = self.current.write().unwrap_or_else(|e| e.into_inner());
        *slot = Some(Preview { token, path, mime });
        token
    }

    fn get(&self, token: u64) -> Option<Preview> {
        let slot = self.current.read().unwrap_or_else(|e| e.into_inner());
        slot.as_ref().filter(|p| p.token == token).cloned()
    }
}

/// The loopback HTTP server that serves the current preview file.
pub struct PreviewServer {
    state: Arc<PreviewState>,
    base_url: String,
}

impl PreviewServer {
    /// Bind to an ephemeral loopback port and start serving. The port is
    /// chosen by the OS, so two launches never collide.
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let state = Arc::new(PreviewState::default());
        let worker = Arc::clone(&state);
        std::thread::spawn(move || accept_loop(listener, worker));
        Ok(Self {
            state,
            base_url: format!("http://127.0.0.1:{port}"),
        })
    }

    /// Offer a file for preview and return the token that addresses it.
    pub fn offer(&self, path: PathBuf, mime: &'static str) -> u64 {
        self.state.offer(path, mime)
    }

    /// The origin the frontend builds its preview URL from.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

/// The most accurate Content-Type for a container.
///
/// ffprobe reports Matroska and WebM as the same `"matroska,webm"`, so the
/// video codec decides which is honest. Over HTTP the label is advisory —
/// GStreamer type-sniffs the bytes — but an accurate one is still correct to
/// send, and costs nothing.
pub fn mime_for(container: &str, video_codec: &str) -> &'static str {
    let lower = container.to_ascii_lowercase();
    let has = |name: &str| lower.split(',').any(|f| f.trim() == name);

    if has("matroska") || has("webm") {
        return match video_codec.to_ascii_lowercase().as_str() {
            "vp8" | "vp9" | "av1" => "video/webm",
            _ => "video/x-matroska",
        };
    }
    if has("mp4") || has("mov") || has("m4a") || has("3gp") || has("3g2") {
        return "video/mp4";
    }
    if has("avi") {
        return "video/x-msvideo";
    }
    if has("mpegts") {
        return "video/mp2t";
    }
    if has("ogg") {
        return "video/ogg";
    }
    if has("flv") {
        return "video/x-flv";
    }
    if has("asf") {
        return "video/x-ms-asf";
    }
    // Deliberately not a guess. Browsers treat octet-stream as "unknown, sniff
    // it", which at worst fails the same way and at best works.
    "application/octet-stream"
}

/// Parse a `Range: bytes=…` header into an inclusive byte range.
///
/// `None` means the range cannot be satisfied and the caller should answer
/// `416`. Only the first range of a multi-range request is honoured — media
/// players ask for one at a time, and a multipart body would be wasted work.
///
/// An open-ended request spans to the end of the file, and the range is
/// deliberately *not* capped: WebKit's `webkitwebsrc` reconstructs the total
/// size as `Content-Length + requested position`, which only yields the true
/// size when the server answers the whole remainder rather than a bounded
/// chunk. Capping the response corrupts the size it reports downstream, and
/// that in turn defeats time-seeks.
pub fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    if len == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?;
    let (from, to) = spec.split(',').next()?.trim().split_once('-')?;

    let (start, end) = match (from.trim(), to.trim()) {
        ("", "") => return None,
        // "-500" means the *last* 500 bytes.
        ("", n) => match n.parse::<u64>().ok()? {
            0 => return None,
            n => (len.saturating_sub(n), len - 1),
        },
        (s, "") => (s.parse().ok()?, len - 1),
        (s, e) => (s.parse().ok()?, e.parse::<u64>().ok()?.min(len - 1)),
    };

    if start > end || start >= len {
        return None;
    }
    Some((start, end))
}

/// Parse the `/<token>` path into the token it names.
fn route(path: &str) -> Option<u64> {
    path.trim_start_matches('/')
        .split('/')
        .next()?
        .parse()
        .ok()
}

fn read_at(file: &mut File, start: u64, len: u64) -> std::io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity(len as usize);
    (&mut *file).take(len).read_to_end(&mut buf)?;
    Ok(buf)
}

/// The streaming counterpart to [`read_at`]: position `file` at `start`, then
/// stream `len` bytes to `out` in bounded chunks. `read_at` buffers the same
/// range into memory instead, for bodies small enough to be worth one write.
fn stream_at<W: Write>(out: &mut W, file: &mut File, start: u64, len: u64) -> std::io::Result<()> {
    file.seek(SeekFrom::Start(start))?;
    stream_file(out, file, len)
}

fn accept_loop(listener: TcpListener, state: Arc<PreviewState>) {
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let state = Arc::clone(&state);
                std::thread::spawn(move || handle(stream, state));
            }
            Err(e) => applog::line("preview", &format!("accept failed: {e}")),
        }
    }
}

fn handle(stream: TcpStream, state: Arc<PreviewState>) {
    if let Err(e) = handle_inner(&stream, &state) {
        applog::line("preview", &format!("connection failed: {e}"));
    }
    let _ = stream.shutdown(Shutdown::Both);
}

fn handle_inner(stream: &TcpStream, state: &PreviewState) -> std::io::Result<()> {
    // A GET/HEAD has no body, so a request is just a request line plus headers
    // ending in a blank line.
    let request = {
        let mut reader = BufReader::new(stream);
        parse_request(&mut reader)?
    };
    let Some((method, path, range)) = request else {
        return Ok(()); // connection closed before a request arrived
    };

    applog::line(
        "preview",
        &format!("{} {} range={}", method, path, range.as_deref().unwrap_or("-")),
    );

    let mut out = stream;
    serve(&mut out, &method, &path, range.as_deref(), state)
}

/// The parsed essentials of one HTTP request. `None` means EOF before any bytes.
fn parse_request(reader: &mut impl BufRead) -> std::io::Result<Option<(String, String, Option<String>)>> {
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(None);
    }
    let mut fields = request_line.split_whitespace();
    let method = fields.next().unwrap_or("").to_ascii_uppercase();
    let path = fields.next().unwrap_or("/").to_string();

    let mut range = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("range") {
                range = Some(value.trim().to_string());
            }
        }
    }
    Ok(Some((method, path, range)))
}

fn base_headers(mime: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Content-Type", mime.to_string()),
        ("Accept-Ranges", "bytes".to_string()),
        ("Cache-Control", "no-store".to_string()),
        ("Connection", "close".to_string()),
    ]
}

/// Write the status line, headers and a blank line.
fn write_head<W: Write>(
    out: &mut W,
    status: &'static str,
    headers: &[(&'static str, String)],
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status}\r\n");
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes())
}

/// Write one complete response: head plus a fully-buffered body.
fn write_response<W: Write>(
    out: &mut W,
    status: &'static str,
    headers: &[(&'static str, String)],
    body: &[u8],
) -> std::io::Result<()> {
    write_head(out, status, headers)?;
    out.write_all(body)?;
    out.flush()
}

fn serve<W: Write>(
    out: &mut W,
    method: &str,
    path: &str,
    range: Option<&str>,
    state: &PreviewState,
) -> std::io::Result<()> {
    let Some(token) = route(path) else {
        applog::line("preview", "-> 404 (unparsable path)");
        return write_response(out, "404 Not Found", &[], &[]);
    };
    let Some(preview) = state.get(token) else {
        // A stale URL for a file that is no longer open, or one we never offered.
        applog::line("preview", "-> 404 (no such token; stale or never offered)");
        return write_response(out, "404 Not Found", &[], &[]);
    };

    let mut file = match File::open(&preview.path) {
        Ok(f) => f,
        Err(e) => {
            applog::line("preview", &format!("-> 404 open failed: {e}"));
            return write_response(out, "404 Not Found", &[], &[]);
        }
    };
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        applog::line("preview", "-> 500 could not stat the file");
        return write_response(out, "500 Internal Server Error", &[], &[]);
    };

    applog::line(
        "preview",
        &format!("serving {} as {} ({len} bytes)", preview.path.display(), preview.mime),
    );

    if method == "HEAD" {
        applog::line("preview", "-> 200 (HEAD, no body)");
        let mut headers = base_headers(preview.mime);
        headers.push(("Content-Length", len.to_string()));
        return write_response(out, "200 OK", &headers, &[]);
    }

    // Resolve the byte range to serve. A plain GET of a large file streams the
    // whole body instead: a 206 first-chunk answer breaks GStreamer's
    // seekability, whereas a 200 with a full Content-Length lets it seek and
    // then follow up with range requests for the parts it must reach.
    let byte_range = match range {
        Some(raw) => match parse_range(raw, len) {
            Some(r) => Some(r),
            None => {
                applog::line("preview", &format!("-> 416 unsatisfiable range {raw:?}"));
                let headers = vec![("Content-Range", format!("bytes */{len}"))];
                return write_response(out, "416 Range Not Satisfiable", &headers, &[]);
            }
        },
        None => None,
    };

    if let Some((start, end)) = byte_range {
        let count = end + 1 - start;
        applog::line("preview", &format!("-> 206 stream {start}-{end}/{len}"));
        let mut headers = base_headers(preview.mime);
        headers.push(("Content-Range", format!("bytes {start}-{end}/{len}")));
        headers.push(("Content-Length", count.to_string()));
        write_head(out, "206 Partial Content", &headers)?;
        return stream_at(out, &mut file, start, count);
    }

    if len <= INLINE_LIMIT {
        return match read_at(&mut file, 0, len) {
            Ok(buf) => {
                applog::line("preview", &format!("-> 200 whole file, {len} bytes"));
                let mut headers = base_headers(preview.mime);
                headers.push(("Content-Length", len.to_string()));
                write_response(out, "200 OK", &headers, &buf)
            }
            Err(e) => {
                applog::line("preview", &format!("-> 500 read failed: {e}"));
                write_response(out, "500 Internal Server Error", &[], &[])
            }
        };
    }

    applog::line("preview", &format!("-> 200 stream, {len} bytes"));
    let mut headers = base_headers(preview.mime);
    headers.push(("Content-Length", len.to_string()));
    write_head(out, "200 OK", &headers)?;
    stream_at(out, &mut file, 0, len)
}

/// Stream `len` bytes of `file` from its current position to `out` in bounded
/// chunks, tolerating a client that stops reading once it has what it needs (a
/// close is not an error). To start at a specific offset, use [`stream_at`].
fn stream_file<W: Write>(out: &mut W, file: &mut File, len: u64) -> std::io::Result<()> {
    let mut remaining = len;
    let mut buf = [0u8; 128 * 1024];
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        let n = match file.read(&mut buf[..want]) {
            Ok(0) => break, // EOF before the advertised length; tolerate it.
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if let Err(e) = out.write_all(&buf[..n]) {
            if is_disconnect(&e) {
                return Ok(());
            }
            return Err(e);
        }
        remaining -= n as u64;
    }
    out.flush()
}

fn is_disconnect(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::UnexpectedEof
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_accurate_type_matches_the_codec() {
        assert_eq!(mime_for("matroska,webm", "h264"), "video/x-matroska");
        assert_eq!(mime_for("matroska,webm", "hevc"), "video/x-matroska");
        assert_eq!(mime_for("matroska,webm", "vp9"), "video/webm");
        assert_eq!(mime_for("matroska,webm", "av1"), "video/webm");
    }

    #[test]
    fn recognises_the_containers_ffprobe_reports() {
        assert_eq!(mime_for("mov,mp4,m4a,3gp,3g2,mj2", "h264"), "video/mp4");
        assert_eq!(mime_for("avi", "mpeg4"), "video/x-msvideo");
        assert_eq!(mime_for("mpegts", "h264"), "video/mp2t");
        assert_eq!(mime_for("flv", "h264"), "video/x-flv");
    }

    #[test]
    fn an_unknown_container_is_not_guessed_at() {
        assert_eq!(mime_for("some-new-format", "h264"), "application/octet-stream");
        assert_eq!(mime_for("", ""), "application/octet-stream");
    }

    #[test]
    fn container_matching_is_case_insensitive_and_whole_token() {
        assert_eq!(mime_for("MATROSKA,WEBM", "H264"), "video/x-matroska");
        // "notmp4" must not match "mp4".
        assert_eq!(mime_for("notmp4", "h264"), "application/octet-stream");
    }

    #[test]
    fn a_route_names_a_file() {
        assert_eq!(route("/7"), Some(7));
        assert_eq!(route("/nope"), None);
        assert_eq!(route("/"), None);
        // A trailing segment is ignored — the URL is token-only.
        assert_eq!(route("/7/anything"), Some(7));
    }

    #[test]
    fn open_ended_range_runs_to_the_end_of_the_file() {
        assert_eq!(parse_range("bytes=0-", 1000), Some((0, 999)));
        assert_eq!(parse_range("bytes=500-", 1000), Some((500, 999)));
    }

    #[test]
    fn closed_range_is_honoured_and_clamped_to_the_file() {
        assert_eq!(parse_range("bytes=10-19", 1000), Some((10, 19)));
        assert_eq!(parse_range("bytes=990-5000", 1000), Some((990, 999)));
    }

    #[test]
    fn suffix_range_counts_back_from_the_end() {
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        // Longer than the file: clamp to the whole file rather than underflow.
        assert_eq!(parse_range("bytes=-5000", 1000), Some((0, 999)));
    }

    #[test]
    fn an_open_ended_range_spans_the_whole_remainder() {
        let huge = 64 * 1024 * 1024 * 1024;
        let (start, end) = parse_range("bytes=0-", huge).expect("satisfiable");
        assert_eq!(end + 1 - start, huge);
    }

    #[test]
    fn unsatisfiable_ranges_are_rejected() {
        assert_eq!(parse_range("bytes=1000-2000", 1000), None);
        assert_eq!(parse_range("bytes=-", 1000), None);
        assert_eq!(parse_range("bytes=abc-def", 1000), None);
        assert_eq!(parse_range("items=0-10", 1000), None);
        assert_eq!(parse_range("bytes=0-", 0), None);
        assert_eq!(parse_range("bytes=-0", 1000), None);
    }

    #[test]
    fn only_the_current_file_is_reachable() {
        let state = PreviewState::default();
        let first = state.offer(PathBuf::from("/v/a.mkv"), "video/x-matroska");
        let second = state.offer(PathBuf::from("/v/b.mp4"), "video/mp4");

        assert!(state.get(second).is_some(), "the open file must be served");
        assert!(state.get(first).is_none(), "a superseded file must not be");
        assert!(state.get(9999).is_none(), "an invented token must not be");
        assert_ne!(first, second, "each file needs its own cache-busting URL");
    }

    /// Run `serve` into a buffer and split the raw response into (head, body).
    fn serve_bytes(
        method: &str,
        path: &str,
        range: Option<&str>,
        state: &PreviewState,
    ) -> (String, Vec<u8>) {
        let mut out = Vec::new();
        serve(&mut out, method, path, range, state).unwrap();
        let marker = b"\r\n\r\n";
        let split = out
            .windows(marker.len())
            .position(|w| w == marker)
            .unwrap_or(out.len());
        let head = String::from_utf8_lossy(&out[..split]).into_owned();
        let body = out[split + marker.len()..].to_vec();
        (head, body)
    }

    #[test]
    fn a_range_response_carries_exactly_the_requested_bytes() {
        let path = std::env::temp_dir().join("whittle-preview-test-range.bin");
        std::fs::write(&path, b"0123456789abcdef").unwrap();

        let state = PreviewState::default();
        let token = state.offer(path.clone(), "application/octet-stream");
        let (head, body) = serve_bytes("GET", &format!("/{token}"), Some("bytes=4-7"), &state);

        assert!(head.starts_with("HTTP/1.1 206 Partial Content"));
        assert!(head.contains("Content-Range: bytes 4-7/16"));
        assert!(head.contains("Content-Length: 4"));
        assert!(head.contains("Content-Type: application/octet-stream"));
        assert_eq!(body, b"4567".to_vec());

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_small_file_without_a_range_is_served_whole() {
        let path = std::env::temp_dir().join("whittle-preview-test-whole.bin");
        std::fs::write(&path, b"hello").unwrap();

        let state = PreviewState::default();
        let token = state.offer(path.clone(), "application/octet-stream");
        let (head, body) = serve_bytes("GET", &format!("/{token}"), None, &state);

        assert!(head.starts_with("HTTP/1.1 200 OK"));
        assert!(head.contains("Content-Length: 5"));
        assert!(head.contains("Accept-Ranges: bytes"));
        assert_eq!(body, b"hello".to_vec());

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_stale_token_is_not_served() {
        let state = PreviewState::default();
        let (head, body) = serve_bytes("GET", "/9999", None, &state);
        assert!(head.starts_with("HTTP/1.1 404 Not Found"));
        assert!(body.is_empty());
    }

    #[test]
    fn streaming_writes_the_whole_file_in_bounded_chunks() {
        let path = std::env::temp_dir().join("whittle-preview-test-stream.bin");
        let data = vec![7u8; 300_000]; // more than one 128 KiB chunk
        std::fs::write(&path, &data).unwrap();

        let mut file = File::open(&path).unwrap();
        let mut out = Vec::new();
        stream_file(&mut out, &mut file, data.len() as u64).unwrap();

        assert_eq!(out, data);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn request_parsing_extracts_the_path_and_range_header() {
        let raw = b"GET /7 HTTP/1.1\r\nHost: 127.0.0.1\r\nRange: bytes=0-99\r\n\r\n";
        let mut reader = BufReader::new(&raw[..]);
        let (method, path, range) = parse_request(&mut reader).unwrap().unwrap();
        assert_eq!(method, "GET");
        assert_eq!(path, "/7");
        assert_eq!(range.as_deref(), Some("bytes=0-99"));
    }
}
