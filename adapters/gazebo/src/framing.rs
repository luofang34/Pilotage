//! Async length-delimited framing for the host<->sidecar bridge connection
//! (ADR-0008).
//!
//! The wire format is a protobuf base-128 varint byte-length prefix followed
//! by the encoded [`crate::wire::BridgeEnvelope`], matching the C++ sidecar's
//! `framing.cpp` and prost's `encode_length_delimited` on the outbound side.
//! This module is I/O-bearing (`adapters/` is exempt from the sans-IO rule,
//! ADR-0002): it is the byte-level read half of the socket.

use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::error::GazeboAdapterError;
use crate::wire::BridgeEnvelope;

/// Upper bound on an inbound envelope body, matching the sidecar's own cap. A
/// raw 320x240 RGB frame is ~230 KB; this leaves headroom while rejecting a
/// corrupt length prefix that would otherwise demand an unbounded allocation.
const MAX_ENVELOPE_BYTES: u64 = 16 * 1024 * 1024;

/// Reads one base-128 varint length prefix, then that many body bytes, and
/// decodes them as a [`BridgeEnvelope`].
///
/// Returns `Ok(None)` on a clean end-of-stream observed before any byte of a
/// frame (the peer closed the connection between frames).
///
/// # Errors
///
/// Returns [`GazeboAdapterError::BridgeRead`] on an I/O error or a truncated
/// frame, and [`GazeboAdapterError::BridgeDecode`] if the body is not a valid
/// envelope. A length prefix over [`MAX_ENVELOPE_BYTES`] is reported as a read
/// error rather than triggering a huge allocation.
pub async fn read_envelope<R>(reader: &mut R) -> Result<Option<BridgeEnvelope>, GazeboAdapterError>
where
    R: AsyncRead + Unpin,
{
    let Some(body_len) = read_varint(reader).await? else {
        return Ok(None);
    };
    if body_len > MAX_ENVELOPE_BYTES {
        return Err(GazeboAdapterError::BridgeRead {
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "bridge envelope length prefix exceeds maximum",
            ),
        });
    }

    let mut body = vec![0_u8; usize_from_u64(body_len)];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|source| GazeboAdapterError::BridgeRead { source })?;

    BridgeEnvelope::decode(body.as_slice())
        .map(Some)
        .map_err(|source| GazeboAdapterError::BridgeDecode { source })
}

/// Reads a base-128 varint from `reader`.
///
/// Returns `Ok(None)` if EOF is seen before the first byte; a partial varint
/// (EOF mid-value) is a truncation error.
async fn read_varint<R>(reader: &mut R) -> Result<Option<u64>, GazeboAdapterError>
where
    R: AsyncRead + Unpin,
{
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    let mut first = true;
    loop {
        let mut byte = [0_u8; 1];
        match reader.read(&mut byte).await {
            Ok(0) => {
                if first {
                    return Ok(None);
                }
                return Err(GazeboAdapterError::BridgeRead {
                    source: std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "eof in the middle of a bridge length prefix",
                    ),
                });
            }
            Ok(_) => {}
            Err(source) => return Err(GazeboAdapterError::BridgeRead { source }),
        }
        first = false;
        value |= u64::from(byte[0] & 0x7F) << shift;
        if byte[0] & 0x80 == 0 {
            return Ok(Some(value));
        }
        shift = shift.wrapping_add(7);
        if shift >= 64 {
            return Err(GazeboAdapterError::BridgeRead {
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "malformed bridge length prefix varint",
                ),
            });
        }
    }
}

/// Narrows a `u64` byte length to `usize` without a `panic`-on-overflow cast.
///
/// On 64-bit hosts this is lossless; the guard only matters on a hypothetical
/// 32-bit target, where an over-large (already `MAX_ENVELOPE_BYTES`-bounded)
/// value would otherwise wrap.
fn usize_from_u64(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::read_envelope;
    use crate::wire::{BridgeEnvelope, BridgeOdometry, bridge_envelope};
    use prost::Message;

    #[tokio::test]
    async fn round_trips_a_length_delimited_odometry_envelope() {
        let envelope = BridgeEnvelope {
            payload: Some(bridge_envelope::Payload::Odometry(BridgeOdometry {
                x: 1.5,
                y: -2.0,
                heading: 0.25,
                speed: 3.0,
                sim_time_ns: 42,
            })),
        };
        let bytes = envelope.encode_length_delimited_to_vec();
        let mut cursor = std::io::Cursor::new(bytes);
        let decoded = read_envelope(&mut cursor)
            .await
            .expect("read succeeds")
            .expect("frame present");
        assert_eq!(decoded, envelope);
    }

    #[tokio::test]
    async fn clean_eof_between_frames_yields_none() {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let decoded = read_envelope(&mut cursor).await.expect("read succeeds");
        assert!(decoded.is_none());
    }
}
