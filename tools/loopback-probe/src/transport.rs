//! WebTransport client connection, loopback-URL validation, and the
//! bootstrap handshake (`ClientHello` -> `ServerWelcome`,
//! `LeaseRequest` -> `LeaseResponse`) over the bidi stream (ADR-0005).

use std::net::IpAddr;

use pilotage_protocol::{
    ClientHello, LeaseRequest, LeaseResponse, ScopeId, ServerWelcome, VehicleId,
};
use wtransport::{ClientConfig, Connection, Endpoint, endpoint::endpoint_side};

use crate::error::ProbeError;
use crate::wire_session::{StreamMessage, decode_one, encode_client_hello, encode_lease_request};

/// The `pilotage.v1` schema version this client claims support for in its
/// `ClientHello`.
const CLIENT_PROTOCOL_VERSION: u32 = pilotage_protocol::SCHEMA_VERSION;
/// Human-readable client identity for diagnostics only (ADR-0005).
const CLIENT_NAME: &str = "loopback-probe";
/// Read-buffer size for one `recv_stream.read` call on the bidi stream;
/// generously oversized against any single handshake message.
const STREAM_READ_BUF_LEN: usize = 4096;

/// Extracts the host portion of an `https://host[:port][/path]` URL,
/// without pulling in a full URL-parsing dependency for this one check.
///
/// Deliberately narrow: this tool only ever needs the host to decide
/// whether `--insecure-loopback` is safe to honor (`validate_loopback_url`);
/// it does not need path, query, or scheme validation, all of which
/// `wtransport::Endpoint::connect` performs itself on the same string.
fn extract_host(url: &str) -> Option<&str> {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host_and_port = after_scheme
        .split_once('/')
        .map_or(after_scheme, |(host, _)| host);
    if let Some(bracketed) = host_and_port.strip_prefix('[') {
        // IPv6 literal: "[::1]:4433" -> "::1".
        return bracketed.split(']').next();
    }
    Some(
        host_and_port
            .split_once(':')
            .map_or(host_and_port, |(host, _)| host),
    )
}

/// Confirms `url`'s host is a loopback address.
///
/// `--insecure-loopback` skips server-certificate verification (see `cli`),
/// which is only safe to combine with a host this process can vouch for by
/// construction: a non-loopback host could be anything, and skipping
/// verification against it would silently accept a spoofed session host.
/// Restricting the flag's effect to loopback hosts keeps "insecure" scoped
/// to "this machine, this process" rather than "the network".
///
/// # Errors
///
/// Returns [`ProbeError::InvalidUrl`] if no host can be extracted from
/// `url`, and [`ProbeError::NonLoopbackHost`] if a host is found but is not
/// a loopback IP or `localhost`.
pub fn validate_loopback_url(url: &str) -> Result<(), ProbeError> {
    let host = extract_host(url).ok_or_else(|| ProbeError::InvalidUrl {
        url: url.to_string(),
        message: "could not extract a host".to_string(),
    })?;
    let is_loopback = host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|addr| addr.is_loopback());
    if is_loopback {
        Ok(())
    } else {
        Err(ProbeError::NonLoopbackHost {
            host: host.to_string(),
        })
    }
}

/// Builds a client endpoint that skips server-certificate verification.
///
/// Only reachable once `validate_loopback_url` has already accepted the
/// target URL's host, so the skipped verification is always paired with a
/// loopback-only target (see that function's doc for why this pairing
/// matters).
///
/// # Errors
///
/// Returns [`ProbeError::Connect`] if the local UDP socket cannot be bound.
pub fn client_endpoint() -> Result<Endpoint<endpoint_side::Client>, ProbeError> {
    let config = ClientConfig::builder()
        .with_bind_default()
        .with_no_cert_validation()
        .build();
    Endpoint::client(config).map_err(|source| ProbeError::Connect {
        message: source.to_string(),
    })
}

/// Connects to `url` and returns the established WebTransport session.
///
/// # Errors
///
/// Returns [`ProbeError::Connect`] if the connection attempt fails.
pub async fn connect(
    endpoint: &Endpoint<endpoint_side::Client>,
    url: &str,
) -> Result<Connection, ProbeError> {
    endpoint
        .connect(url)
        .await
        .map_err(|source| ProbeError::Connect {
            message: source.to_string(),
        })
}

/// The outcome of the bootstrap handshake: the host's welcome plus the
/// granted lease's fencing generation.
pub struct HandshakeOutcome {
    /// The host's `ServerWelcome` reply.
    pub welcome: ServerWelcome,
    /// The granted `LeaseResponse` for `vehicle.motion`.
    pub lease: LeaseResponse,
}

/// Opens the bootstrap bidi stream and drives `ClientHello` ->
/// `ServerWelcome`, then `LeaseRequest` -> `LeaseResponse` for
/// `(vehicle, "vehicle.motion")`.
///
/// # Errors
///
/// Returns [`ProbeError::BidiStream`] if the stream cannot be opened or
/// written to, [`ProbeError::Decode`] if a reply fails to parse, and
/// [`ProbeError::Protocol`] if a reply is the wrong message type or the
/// lease was denied.
pub async fn handshake(
    connection: &Connection,
    vehicle: VehicleId,
) -> Result<
    (
        HandshakeOutcome,
        wtransport::SendStream,
        wtransport::RecvStream,
    ),
    ProbeError,
> {
    let (mut send, mut recv) = open_bidi(connection).await?;

    let mut buf = Vec::new();

    let hello = ClientHello {
        protocol_version: CLIENT_PROTOCOL_VERSION,
        client_name: CLIENT_NAME.to_string(),
        join_token: Vec::new(),
    };
    write_frame(&mut send, &encode_client_hello(&hello)).await?;
    let welcome = match read_message(&mut recv, &mut buf).await? {
        StreamMessage::ServerWelcome(welcome) => welcome,
        other => {
            return Err(ProbeError::Protocol {
                message: format!("expected ServerWelcome, got {other:?}"),
            });
        }
    };

    let scope = ScopeId::new("vehicle.motion");
    let request = LeaseRequest { vehicle, scope };
    write_frame(&mut send, &encode_lease_request(&request)).await?;
    let lease = match read_message(&mut recv, &mut buf).await? {
        StreamMessage::LeaseResponse(response) => response,
        other => {
            return Err(ProbeError::Protocol {
                message: format!("expected LeaseResponse, got {other:?}"),
            });
        }
    };
    if !lease.granted {
        return Err(ProbeError::Protocol {
            message: format!("vehicle.motion lease denied: {:?}", lease.reason),
        });
    }

    Ok((HandshakeOutcome { welcome, lease }, send, recv))
}

/// Opens the bootstrap bidi stream, mapping the `wtransport` open/await
/// error into this binary's typed error.
async fn open_bidi(
    connection: &Connection,
) -> Result<(wtransport::SendStream, wtransport::RecvStream), ProbeError> {
    connection
        .open_bi()
        .await
        .map_err(|source| ProbeError::BidiStream {
            message: source.to_string(),
        })?
        .await
        .map_err(|source| ProbeError::BidiStream {
            message: source.to_string(),
        })
}

/// Writes one length-delimited envelope frame to the bidi send stream.
async fn write_frame(send: &mut wtransport::SendStream, frame: &[u8]) -> Result<(), ProbeError> {
    send.write_all(frame)
        .await
        .map_err(|source| ProbeError::BidiStream {
            message: source.to_string(),
        })
}

/// Reads bytes from the bidi recv stream until one full envelope frame is
/// available, decodes it, and returns the typed message.
///
/// `buf` is the caller's persisted reassembly buffer, carried across calls:
/// a `recv.read()` can return more than one envelope's worth of bytes (e.g.
/// `ServerWelcome` and a following `LeaseResponse` arriving in the same
/// read), and any bytes trailing the decoded envelope must survive for the
/// next call rather than be dropped with a fresh empty buffer, or the
/// trailing message would be silently lost (mirrors `receiver.rs`'s
/// `stream_buf` reassembly).
async fn read_message(
    recv: &mut wtransport::RecvStream,
    buf: &mut Vec<u8>,
) -> Result<StreamMessage, ProbeError> {
    loop {
        if let Ok((message, consumed)) = decode_one(buf) {
            buf.drain(..consumed);
            return Ok(message);
        }
        let mut chunk = [0u8; STREAM_READ_BUF_LEN];
        let read = recv
            .read(&mut chunk)
            .await
            .map_err(|source| ProbeError::BidiStream {
                message: source.to_string(),
            })?
            .ok_or_else(|| ProbeError::BidiStream {
                message: "bidi stream closed before a complete reply arrived".to_string(),
            })?;
        buf.extend_from_slice(&chunk[..read]);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{extract_host, validate_loopback_url};

    #[test]
    fn extract_host_strips_scheme_port_and_path() {
        assert_eq!(
            extract_host("https://127.0.0.1:4433/session"),
            Some("127.0.0.1")
        );
        assert_eq!(extract_host("https://example.com"), Some("example.com"));
    }

    #[test]
    fn extract_host_handles_ipv6_brackets() {
        assert_eq!(extract_host("https://[::1]:4433"), Some("::1"));
    }

    #[test]
    fn accepts_ipv4_loopback() {
        validate_loopback_url("https://127.0.0.1:4433").expect("loopback accepted");
    }

    #[test]
    fn accepts_ipv6_loopback() {
        validate_loopback_url("https://[::1]:4433").expect("loopback accepted");
    }

    #[test]
    fn accepts_localhost_hostname() {
        validate_loopback_url("https://localhost:4433").expect("localhost accepted");
    }

    #[test]
    fn rejects_remote_host() {
        let err = validate_loopback_url("https://example.com:4433").expect_err("should reject");
        assert!(matches!(err, super::ProbeError::NonLoopbackHost { .. }));
    }

    #[test]
    fn rejects_remote_ip() {
        let err = validate_loopback_url("https://8.8.8.8:4433").expect_err("should reject");
        assert!(matches!(err, super::ProbeError::NonLoopbackHost { .. }));
    }

    #[test]
    fn rejects_malformed_url() {
        assert!(validate_loopback_url("not a url").is_err());
    }
}
