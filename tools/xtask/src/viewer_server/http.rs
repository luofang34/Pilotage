//! The HTTP/1.0 subset the viewer needs: GET and HEAD of static files,
//! one request per connection.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The largest request head accepted. Viewer requests are a request line
/// and a handful of browser headers.
const MAX_HEAD_BYTES: usize = 16 * 1024;

/// Reads a request head and returns its request line, or `None` when the
/// peer closed, stalled, or sent an oversized head.
pub(super) fn read_request_line(stream: &mut impl Read) -> Option<String> {
    let mut head = Vec::new();
    let mut chunk = [0_u8; 2048];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 || head.len() + read > MAX_HEAD_BYTES {
            return None;
        }
        head.extend_from_slice(&chunk[..read]);
    }
    let text = String::from_utf8_lossy(&head);
    text.lines().next().map(str::to_owned)
}

/// A response ready to be written.
pub(super) struct Response {
    /// The status code.
    pub(super) status: u16,
    reason: &'static str,
    content_type: &'static str,
    extra_header: Option<(&'static str, String)>,
    body: Body,
    head_only: bool,
}

enum Body {
    Text(&'static str),
    File { file: File, len: u64 },
}

impl Response {
    fn error(status: u16, reason: &'static str) -> Self {
        Self {
            status,
            reason,
            content_type: "text/plain; charset=utf-8",
            extra_header: None,
            body: Body::Text(reason),
            head_only: false,
        }
    }

    /// Writes the status line, headers and (unless HEAD) the body.
    pub(super) fn write_to(self, stream: &mut impl Write) -> std::io::Result<()> {
        let len = match &self.body {
            Body::Text(text) => text.len() as u64,
            Body::File { len, .. } => *len,
        };
        // Cache-Control: no-store on every response: the viewer is served
        // straight from the working tree, and any heuristic caching would
        // let the browser run a client that has diverged from the source
        // on disk.
        let mut head = format!(
            "HTTP/1.0 {} {}\r\nContent-Type: {}\r\nContent-Length: {len}\r\n\
             Cache-Control: no-store\r\nConnection: close\r\n",
            self.status, self.reason, self.content_type
        );
        if let Some((name, value)) = &self.extra_header {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes())?;
        if !self.head_only {
            match self.body {
                Body::Text(text) => stream.write_all(text.as_bytes())?,
                Body::File { mut file, .. } => {
                    std::io::copy(&mut file, stream)?;
                }
            }
        }
        stream.flush()
    }
}

/// Answers one request line against `root`, which must be canonical.
pub(super) fn respond(request_line: &str, root: &Path) -> Response {
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Response::error(400, "Bad Request");
    };
    let head_only = match method {
        "GET" => false,
        "HEAD" => true,
        _ => {
            let mut response = Response::error(405, "Method Not Allowed");
            response.extra_header = Some(("Allow", "GET, HEAD".to_owned()));
            return response;
        }
    };
    let mut response = respond_to_target(target, root);
    response.head_only = head_only;
    response
}

fn respond_to_target(target: &str, root: &Path) -> Response {
    let without_fragment = target.split('#').next().unwrap_or_default();
    let (raw_path, query) = match without_fragment.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (without_fragment, None),
    };
    if !raw_path.starts_with('/') {
        return Response::error(400, "Bad Request");
    }
    let Some(path) = percent_decode(raw_path) else {
        return Response::error(400, "Bad Request");
    };
    match resolve(root, &path) {
        Resolved::File(file_path) => file_response(&file_path),
        Resolved::Directory => {
            // Relative asset URLs in a directory's index resolve against
            // the trailing slash, so `/clients/web` must become
            // `/clients/web/` before the index is served.
            let location = match query {
                Some(query) => format!("{raw_path}/?{query}"),
                None => format!("{raw_path}/"),
            };
            let mut response = Response::error(301, "Moved Permanently");
            response.extra_header = Some(("Location", location));
            response
        }
        Resolved::NotFound => Response::error(404, "Not Found"),
    }
}

enum Resolved {
    File(PathBuf),
    /// A directory addressed without its trailing slash.
    Directory,
    NotFound,
}

/// Maps a decoded URL path onto a file under `root`. Canonicalizing
/// resolves `..` and symlinks, so the prefix check is what keeps every
/// served file inside the tree.
fn resolve(root: &Path, path: &str) -> Resolved {
    let candidate = root.join(path.trim_start_matches('/'));
    let Some(canonical) = contained(root, &candidate) else {
        return Resolved::NotFound;
    };
    if canonical.is_file() {
        return Resolved::File(canonical);
    }
    if !canonical.is_dir() {
        return Resolved::NotFound;
    }
    if !path.ends_with('/') {
        return Resolved::Directory;
    }
    match contained(root, &canonical.join("index.html")) {
        Some(index) if index.is_file() => Resolved::File(index),
        _ => Resolved::NotFound,
    }
}

fn contained(root: &Path, candidate: &Path) -> Option<PathBuf> {
    candidate
        .canonicalize()
        .ok()
        .filter(|canonical| canonical.starts_with(root))
}

fn file_response(path: &Path) -> Response {
    let opened = File::open(path).and_then(|file| Ok((file.metadata()?.len(), file)));
    let Ok((len, file)) = opened else {
        return Response::error(404, "Not Found");
    };
    Response {
        status: 200,
        reason: "OK",
        content_type: content_type(path),
        extra_header: None,
        body: Body::File { file, len },
        head_only: false,
    }
}

/// The media type for a file's extension. `application/wasm` matters:
/// `WebAssembly.instantiateStreaming` refuses any other type.
pub(super) fn content_type(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json" | "map") => "application/json",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        Some("woff2") => "font/woff2",
        Some("pbf") => "application/x-protobuf",
        _ => "application/octet-stream",
    }
}

/// Decodes `%XX` escapes; `None` for a malformed escape, a NUL byte, or
/// a result that is not UTF-8.
fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'%' {
            let hex = bytes.get(index + 1..index + 3)?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            decoded.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            index += 3;
        } else {
            decoded.push(byte);
            index += 1;
        }
    }
    if decoded.contains(&0) {
        return None;
    }
    String::from_utf8(decoded).ok()
}
