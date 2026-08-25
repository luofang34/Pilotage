use std::io::{Read, Write as _};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::AviateSupervisorError;

const MAX_MESSAGE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArmedMessage {
    pub(crate) correlation_nonce: flight_tune::Digest,
    pub(crate) spawn_intent_digest: flight_tune::Digest,
    pub(crate) process_identity_digest: flight_tune::Digest,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReleaseMessage {
    pub(crate) correlation_nonce: flight_tune::Digest,
    pub(crate) release_secret: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetReadyMessage {
    pub(crate) correlation_nonce: flight_tune::Digest,
    pub(crate) process_identity_digest: flight_tune::Digest,
    pub(crate) target_attestation_digest: flight_tune::Digest,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub(crate) enum TargetReleaseMessage {
    Ready {
        correlation_nonce: flight_tune::Digest,
        process_identity_digest: flight_tune::Digest,
        target_attestation_digest: flight_tune::Digest,
    },
    RejectedIdentityMismatch {
        correlation_nonce: flight_tune::Digest,
        detail: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub(crate) enum GateEvent {
    TargetStarted { pid: u32 },
    TargetContained { pid: u32 },
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn connect_and_write<T: Serialize>(
    path: &Path,
    message: &T,
    timeout: Duration,
) -> Result<(), AviateSupervisorError> {
    let stream = std::os::unix::net::UnixStream::connect(path)
        .map_err(|source| protocol_io("connect to supervisor socket", source))?;
    write_message(stream, message, timeout)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn read_message_until_blocking<R: for<'de> Deserialize<'de>>(
    mut stream: std::os::unix::net::UnixStream,
    deadline: std::time::Instant,
) -> Result<R, AviateSupervisorError> {
    stream
        .set_nonblocking(true)
        .map_err(|source| protocol_io("configure supervisor message read", source))?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => return decode(&bytes),
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.len() as u64 > MAX_MESSAGE_BYTES {
                    return Err(AviateSupervisorError::protocol(
                        "the supervisor message is too large",
                    ));
                }
            }
            Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    return Err(AviateSupervisorError::Timeout {
                        operation: "read owner readiness",
                    });
                }
                std::thread::park_timeout(Duration::from_millis(1));
            }
            Err(source) => return Err(protocol_io("read supervisor message", source)),
        }
    }
}

pub(crate) fn decode<R: for<'de> Deserialize<'de>>(
    bytes: &[u8],
) -> Result<R, AviateSupervisorError> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_MESSAGE_BYTES {
        return Err(AviateSupervisorError::protocol(
            "the supervisor message has an invalid length",
        ));
    }
    serde_json::from_slice(bytes)
        .map_err(|source| AviateSupervisorError::json_protocol("decoding", source))
}

pub(crate) fn encode_line<T: Serialize>(message: &T) -> Result<Vec<u8>, AviateSupervisorError> {
    let mut bytes = serde_json::to_vec(message)
        .map_err(|source| AviateSupervisorError::json_protocol("encoding", source))?;
    if bytes.is_empty() || bytes.len() as u64 >= MAX_MESSAGE_BYTES {
        return Err(AviateSupervisorError::protocol(
            "the supervisor message has an invalid length",
        ));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

pub(crate) fn read_line_blocking(reader: &mut impl Read) -> Result<Vec<u8>, AviateSupervisorError> {
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let count = reader
            .read(&mut byte)
            .map_err(|source| protocol_io("read anonymous process pipe", source))?;
        if count == 0 {
            return Err(AviateSupervisorError::protocol(
                "an anonymous process pipe closed before one message",
            ));
        }
        if byte[0] == b'\n' {
            return Ok(bytes);
        }
        bytes.push(byte[0]);
        if bytes.len() as u64 > MAX_MESSAGE_BYTES {
            return Err(AviateSupervisorError::protocol(
                "an anonymous process pipe message is too large",
            ));
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_message<T: Serialize>(
    mut stream: std::os::unix::net::UnixStream,
    message: &T,
    timeout: Duration,
) -> Result<(), AviateSupervisorError> {
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|source| protocol_io("configure supervisor write timeout", source))?;
    let bytes = serde_json::to_vec(message)
        .map_err(|source| AviateSupervisorError::json_protocol("encoding", source))?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_MESSAGE_BYTES {
        return Err(AviateSupervisorError::protocol(
            "the supervisor message has an invalid length",
        ));
    }
    stream
        .write_all(&bytes)
        .map_err(|source| protocol_io("write supervisor message", source))?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|source| protocol_io("finish supervisor message", source))
}

fn protocol_io(operation: &'static str, source: std::io::Error) -> AviateSupervisorError {
    AviateSupervisorError::ProcessIo { operation, source }
}
