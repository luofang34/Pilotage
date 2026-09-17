//! The small part of HTTP/1.1 that the gateway speaks: one request for each
//! connection, a body with a `Content-Length`, and a JSON answer.

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The largest request head.
const MAX_HEAD_BYTES: usize = 16 * 1024;
/// The largest request body. A request can carry image frames.
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
/// The most header lines that a request can have.
const MAX_HEADERS: usize = 48;

/// Why a request cannot be read.
#[derive(Debug, thiserror::Error)]
pub(super) enum RequestError {
    /// The connection failed.
    #[error("the connection failed")]
    Io(#[source] std::io::Error),
    /// The client closed before it sent a complete request.
    #[error("the client closed before the request was complete")]
    Closed,
    /// The request head is not HTTP/1.1.
    #[error("the request head is not valid HTTP")]
    Head(#[source] httparse::Error),
    /// A part of the request is larger than the gateway accepts.
    #[error("the request {part} is larger than {limit} bytes")]
    TooLarge {
        /// The part of the request.
        part: &'static str,
        /// The limit in bytes.
        limit: usize,
    },
    /// The `Content-Length` header is not a number.
    #[error("the Content-Length header is not a number")]
    ContentLength,
}

/// One parsed request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Request {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) origin: Option<String>,
    pub(super) body: Vec<u8>,
}

/// One answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Response {
    status: u16,
    body: Vec<u8>,
    preflight: bool,
}

impl Response {
    /// A JSON answer. A value that does not encode becomes a fault line.
    pub(super) fn json(status: u16, value: &impl Serialize) -> Self {
        match serde_json::to_vec(value) {
            Ok(body) => Self {
                status,
                body,
                preflight: false,
            },
            Err(error) => Self::fault(500, &format!("the answer did not encode: {error}")),
        }
    }

    /// A fault line: `{"error": ...}`.
    pub(super) fn fault(status: u16, detail: &str) -> Self {
        let body = serde_json::json!({ "error": detail })
            .to_string()
            .into_bytes();
        Self {
            status,
            body,
            preflight: false,
        }
    }

    /// The answer to a CORS preflight.
    pub(super) const fn preflight() -> Self {
        Self {
            status: 204,
            body: Vec::new(),
            preflight: true,
        }
    }
}

/// Reads one request: the head, then a body of `Content-Length` bytes.
pub(super) async fn read_request<S>(stream: &mut S) -> Result<Request, RequestError>
where
    S: AsyncReadExt + Unpin,
{
    let mut buffer = Vec::with_capacity(1024);
    let (mut request, head_len, body_len) = loop {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await.map_err(RequestError::Io)?;
        if read == 0 {
            return Err(RequestError::Closed);
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(parsed) = parse_head(&buffer)? {
            break parsed;
        }
        if buffer.len() > MAX_HEAD_BYTES {
            return Err(RequestError::TooLarge {
                part: "head",
                limit: MAX_HEAD_BYTES,
            });
        }
    };
    if body_len > MAX_BODY_BYTES {
        return Err(RequestError::TooLarge {
            part: "body",
            limit: MAX_BODY_BYTES,
        });
    }
    let mut body = buffer.split_off(head_len);
    body.truncate(body_len);
    let missing = body_len - body.len();
    if missing > 0 {
        let start = body.len();
        body.resize(body_len, 0);
        stream
            .read_exact(&mut body[start..])
            .await
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::UnexpectedEof => RequestError::Closed,
                _ => RequestError::Io(error),
            })?;
    }
    request.body = body;
    Ok(request)
}

/// Parses a complete head. `None` means that more bytes are needed. The
/// values are the request with no body, the head length and the body length.
fn parse_head(buffer: &[u8]) -> Result<Option<(Request, usize, usize)>, RequestError> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut parsed = httparse::Request::new(&mut headers);
    let head_len = match parsed.parse(buffer).map_err(RequestError::Head)? {
        httparse::Status::Complete(length) => length,
        httparse::Status::Partial => return Ok(None),
    };
    let header = |name: &str| {
        parsed
            .headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .and_then(|header| std::str::from_utf8(header.value).ok())
            .map(str::trim)
    };
    let body_len = match header("content-length") {
        Some(text) => text.parse().map_err(|_| RequestError::ContentLength)?,
        None => 0,
    };
    let request = Request {
        method: parsed.method.unwrap_or_default().to_owned(),
        path: parsed.path.unwrap_or_default().to_owned(),
        origin: header("origin").map(str::to_owned),
        body: Vec::new(),
    };
    Ok(Some((request, head_len, body_len)))
}

/// Writes one answer and closes the exchange. `origin` is the permitted page
/// origin to grant, when the request came from a page.
pub(super) async fn write_response<S>(
    stream: &mut S,
    response: &Response,
    origin: Option<&str>,
) -> std::io::Result<()>
where
    S: AsyncWriteExt + Unpin,
{
    let reason = match response.status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    let mut head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
        response.status,
        response.body.len()
    );
    if !response.body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    if let Some(origin) = origin {
        head.push_str(&format!(
            "Access-Control-Allow-Origin: {origin}\r\nVary: Origin\r\n"
        ));
        if response.preflight {
            head.push_str(
                "Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
                 Access-Control-Allow-Headers: Content-Type\r\n\
                 Access-Control-Max-Age: 600\r\n",
            );
        }
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&response.body).await?;
    stream.shutdown().await
}
