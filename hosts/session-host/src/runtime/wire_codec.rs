//! Envelope encode/decode the driver owns so [`pilotage_session::SessionEngine`]
//! stays sans-IO (ADR-0002, ADR-0005, ADR-0014).
//!
//! `pilotage-protocol` supplies the domain<->wire conversions and the
//! length-delimited/datagram framing helpers; this module is the one place
//! that picks which wire arm each [`DomainEnvelope`] or [`OutboundMessage`]
//! maps to and drives `prost` encode/decode.

use pilotage_protocol::wire;
use pilotage_protocol::{
    ClientHello, DecodeError, LeaseRequest, Ping, SCHEMA_VERSION, ScopedControlFrame,
    decode_control_frame_envelope, decode_envelope_length_delimited,
    encode_envelope_length_delimited,
};
use pilotage_session::{DomainEnvelope, OutboundMessage};
use prost::Message;
use prost::encoding::decode_varint;

/// Upper bound on a single client-origin bootstrap-stream frame.
///
/// The only frames a client legitimately sends on this stream are
/// `ClientHello` and `LeaseRequest` (ADR-0005), both a few hundred bytes at
/// most. Without a cap, a client declaring a huge varint length and then
/// dribbling the body would grow the reassembly buffer toward that declared
/// size before a full frame ever arrives — an unbounded-memory vector that
/// QUIC flow control does not bound, because the application buffer is
/// separate from the transport window. 64 KiB leaves generous headroom while
/// keeping a hostile declaration cheap to reject.
pub const MAX_BOOTSTRAP_FRAME_LEN: usize = 64 * 1024;

/// A client declared a bootstrap-stream frame larger than
/// [`MAX_BOOTSTRAP_FRAME_LEN`]; the connection must be closed rather than
/// buffered toward the declared size.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("bootstrap frame declares {declared} bytes, over the {MAX_BOOTSTRAP_FRAME_LEN}-byte limit")]
pub struct OversizedFrame {
    /// The frame length the client's varint prefix announced.
    pub declared: usize,
}

/// Inspects the length-delimited varint prefix at the front of `bytes`
/// without consuming from the connection's real read buffer, so the caller
/// can tell "not enough bytes buffered yet" apart from "malformed frame" and
/// "hostile oversized declaration" before attempting a full decode.
///
/// Returns `Ok(Some(total_frame_len))` — the varint prefix length plus the
/// payload it announces — once that many bytes are available; `Ok(None)` if
/// the prefix itself is incomplete or the announced frame has not fully
/// arrived yet; and [`OversizedFrame`] the moment the declared length exceeds
/// [`MAX_BOOTSTRAP_FRAME_LEN`], so the caller closes the connection instead of
/// growing the buffer toward an attacker-chosen size.
pub fn complete_frame_len(bytes: &[u8]) -> Result<Option<usize>, OversizedFrame> {
    let mut cursor: &[u8] = bytes;
    let before = cursor.len();
    let Ok(payload_len) = decode_varint(&mut cursor) else {
        return Ok(None);
    };
    let prefix_len = before - cursor.len();
    let Ok(payload_len) = usize::try_from(payload_len) else {
        return Err(OversizedFrame {
            declared: usize::MAX,
        });
    };
    if payload_len > MAX_BOOTSTRAP_FRAME_LEN {
        return Err(OversizedFrame {
            declared: payload_len,
        });
    }
    let Some(total) = prefix_len.checked_add(payload_len) else {
        return Ok(None);
    };
    Ok((bytes.len() >= total).then_some(total))
}

/// Errors decoding a length-delimited bootstrap-stream envelope into a
/// [`DomainEnvelope`].
#[derive(Debug, thiserror::Error)]
pub enum BootstrapDecodeError {
    /// The bytes did not decode as a valid envelope.
    #[error(transparent)]
    Decode(#[from] DecodeError),
    /// The envelope decoded but carried a payload arm the bootstrap stream
    /// never legitimately receives from a client (for example a server-only
    /// message, or an arm this build does not recognize).
    #[error("unexpected bootstrap-stream payload arm")]
    UnexpectedPayload,
    /// The envelope's `schema_version` is not one this build accepts.
    #[error("unsupported schema_version {found} (expected {expected})")]
    UnsupportedSchemaVersion {
        /// The schema version this build produces and accepts.
        expected: u32,
        /// The schema version found on the envelope.
        found: u32,
    },
}

/// Decodes one length-delimited envelope from the front of `bytes` into a
/// client-origin [`DomainEnvelope`] plus the unconsumed remainder.
///
/// Only `ClientHello`, `LeaseRequest`, `LeaseRelease`, `ProfileActivation`,
/// and `ControlActionCommand` are legitimately received on the bootstrap
/// stream (ADR-0005): control frames and `Ping` travel as datagrams —
/// discrete actions deliberately do NOT (CTRL-01 reliable delivery).
///
/// # Errors
///
/// Returns [`BootstrapDecodeError`] if the bytes are not a valid envelope,
/// the schema version is unsupported, or the payload is not one of the two
/// client-origin bootstrap-stream arms.
pub fn decode_bootstrap_message(
    bytes: &[u8],
) -> Result<(DomainEnvelope, &[u8]), BootstrapDecodeError> {
    let (envelope, rest) = decode_envelope_length_delimited(bytes)?;
    if envelope.schema_version != SCHEMA_VERSION {
        return Err(BootstrapDecodeError::UnsupportedSchemaVersion {
            expected: SCHEMA_VERSION,
            found: envelope.schema_version,
        });
    }
    let domain = match envelope.payload {
        Some(wire::envelope::Payload::ClientHello(hello)) => {
            DomainEnvelope::Hello(ClientHello::from(hello))
        }
        Some(wire::envelope::Payload::LeaseRequest(request)) => {
            DomainEnvelope::Lease(LeaseRequest::try_from(request).map_err(DecodeError::from)?)
        }
        Some(wire::envelope::Payload::LeaseRelease(release)) => DomainEnvelope::Release(
            pilotage_protocol::LeaseRelease::try_from(release).map_err(DecodeError::from)?,
        ),
        Some(wire::envelope::Payload::ProfileActivation(activation)) => {
            DomainEnvelope::ProfileActivation(
                pilotage_protocol::ProfileActivation::try_from(activation)
                    .map_err(DecodeError::from)?,
            )
        }
        Some(wire::envelope::Payload::ControlActionCommand(command)) => {
            DomainEnvelope::ActionCommand(
                pilotage_protocol::ControlActionCommand::try_from(command)
                    .map_err(DecodeError::from)?,
            )
        }
        _ => return Err(BootstrapDecodeError::UnexpectedPayload),
    };
    Ok((domain, rest))
}

/// Decodes one control-fast datagram payload into a [`DomainEnvelope::Frame`].
///
/// # Errors
///
/// Returns [`DecodeError`] if the bytes are not a valid `ControlFrame`
/// envelope.
pub fn decode_control_datagram(bytes: &[u8]) -> Result<ScopedControlFrame, DecodeError> {
    decode_control_frame_envelope(bytes)
}

/// Decodes one datagram payload as a `Ping` envelope, for the RTT probe
/// channel (ADR-0009).
///
/// # Errors
///
/// Returns [`BootstrapDecodeError`] if the bytes are not a valid envelope, an
/// unsupported schema version, or not a `Ping` payload.
pub fn decode_ping_datagram(bytes: &[u8]) -> Result<Ping, BootstrapDecodeError> {
    let envelope = wire::Envelope::decode(bytes).map_err(|source| {
        BootstrapDecodeError::Decode(DecodeError::Prost {
            message: "pilotage.v1.Envelope",
            source,
        })
    })?;
    if envelope.schema_version != SCHEMA_VERSION {
        return Err(BootstrapDecodeError::UnsupportedSchemaVersion {
            expected: SCHEMA_VERSION,
            found: envelope.schema_version,
        });
    }
    match envelope.payload {
        Some(wire::envelope::Payload::Ping(ping)) => {
            Ok(Ping::try_from(ping).map_err(DecodeError::from)?)
        }
        _ => Err(BootstrapDecodeError::UnexpectedPayload),
    }
}

/// Encodes an [`OutboundMessage`] as a length-delimited envelope. Used for
/// both the bootstrap stream (`Welcome`/`LeaseResponse`) and the dedicated
/// authority-events stream (`Authority`) — the framing is identical, only
/// the destination stream differs (ADR-0005).
#[must_use]
pub fn encode_envelope_message(message: &OutboundMessage) -> Vec<u8> {
    let payload = match message {
        OutboundMessage::Welcome(welcome) => wire::envelope::Payload::ServerWelcome(welcome.into()),
        OutboundMessage::LeaseResponse(response) => {
            wire::envelope::Payload::LeaseResponse(response.into())
        }
        OutboundMessage::LeaseReleased(released) => {
            wire::envelope::Payload::LeaseReleased(released.into())
        }
        OutboundMessage::LinkLossCleared(cleared) => {
            wire::envelope::Payload::LinkLossCleared(cleared.into())
        }
        OutboundMessage::Pong(pong) => wire::envelope::Payload::Pong(pong.into()),
        OutboundMessage::ControlActionResult(result) => {
            wire::envelope::Payload::ControlActionResult(result.into())
        }
        OutboundMessage::Authority(effect) => {
            wire::envelope::Payload::AuthorityEvent(wire::AuthorityEvent::from(effect))
        }
    };
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(payload),
    };
    encode_envelope_length_delimited(&envelope)
}

/// Encodes a `Pong` as a standalone envelope for a control-fast datagram.
#[must_use]
pub fn encode_pong_datagram(pong: &pilotage_protocol::Pong) -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::Pong(pong.into())),
    };
    envelope.encode_to_vec()
}

/// Encodes a telemetry sample as a standalone envelope for a telemetry-fast
/// datagram.
#[must_use]
pub fn encode_telemetry_datagram(sample: wire::TelemetrySample) -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::TelemetrySample(sample)),
    };
    envelope.encode_to_vec()
}

/// Encodes a `FrameRejected` notice as a datagram envelope (sent back to the
/// frame's sender only, ADR-0009).
#[must_use]
pub fn encode_frame_rejected_datagram(rejection: &pilotage_protocol::FrameRejected) -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::FrameRejected(rejection.into())),
    };
    envelope.encode_to_vec()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{MAX_BOOTSTRAP_FRAME_LEN, complete_frame_len};
    use prost::encoding::encode_varint;

    #[test]
    fn incomplete_prefix_is_not_yet_a_frame() {
        // A single 0x80 continuation byte is an unfinished varint.
        assert_eq!(complete_frame_len(&[0x80]), Ok(None));
    }

    #[test]
    fn full_frame_reports_total_length() {
        let mut bytes = Vec::new();
        encode_varint(3, &mut bytes);
        bytes.extend_from_slice(&[1, 2, 3]);
        let total = bytes.len();
        assert_eq!(complete_frame_len(&bytes), Ok(Some(total)));
    }

    #[test]
    fn declared_body_not_yet_arrived_is_none() {
        let mut bytes = Vec::new();
        encode_varint(3, &mut bytes);
        bytes.push(1); // only one of three body bytes buffered
        assert_eq!(complete_frame_len(&bytes), Ok(None));
    }

    #[test]
    fn oversized_declaration_is_rejected_before_buffering() {
        // A hostile client announces a body far larger than the cap while
        // sending almost no bytes; the guard must reject on the prefix alone,
        // never wait for the declared bytes to arrive.
        let mut bytes = Vec::new();
        let declared = (MAX_BOOTSTRAP_FRAME_LEN as u64) + 1;
        encode_varint(declared, &mut bytes);
        let err = complete_frame_len(&bytes).expect_err("over-cap frame must be rejected");
        assert_eq!(err.declared, MAX_BOOTSTRAP_FRAME_LEN + 1);
    }

    #[test]
    fn frame_exactly_at_the_cap_is_allowed() {
        let mut bytes = Vec::new();
        encode_varint(MAX_BOOTSTRAP_FRAME_LEN as u64, &mut bytes);
        // Prefix only; body has not arrived, so this is a well-formed pending
        // frame (None), not a rejection.
        assert_eq!(complete_frame_len(&bytes), Ok(None));
    }
}
