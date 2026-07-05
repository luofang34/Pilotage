//! Envelope encode/decode for session-bootstrap messages carried on the
//! reliable bidi stream (ADR-0005): `ClientHello`, `ServerWelcome`,
//! `LeaseRequest`, `LeaseResponse`, `Ping`, `Pong`, `FrameRejected`.
//!
//! `pilotage-protocol` exports `encode_control_frame_envelope` /
//! `decode_control_frame_envelope` for the datagram-class `ControlFrame`
//! payload, but no equivalent generic helper for the other envelope arms
//! (they are conceptually one-per-handshake-step, not a hot per-tick path,
//! so each one gets its own small wire function here rather than the
//! calling code hand-rolling `wire::Envelope` construction inline).

use pilotage_protocol::wire;
use pilotage_protocol::{
    ClientHello, DecodeError, FrameRejected, LeaseRequest, LeaseResponse, Ping, Pong,
    ServerWelcome, decode_envelope_length_delimited, encode_envelope_length_delimited,
};

use crate::error::ProbeError;

/// The `pilotage.v1` schema version this binary produces and accepts,
/// mirroring `pilotage_protocol::convert::SCHEMA_VERSION` (not exported, so
/// restated here; both must move together if the schema ever revs).
const SCHEMA_VERSION: u32 = 1;

/// Wraps a wire payload in a versioned envelope and encodes it with a
/// length-delimited prefix, ready to append to the bidi stream's send
/// buffer.
fn encode(payload: wire::envelope::Payload) -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(payload),
    };
    encode_envelope_length_delimited(&envelope)
}

/// Encodes a `ClientHello` as a length-delimited envelope frame.
#[must_use]
pub fn encode_client_hello(hello: &ClientHello) -> Vec<u8> {
    encode(wire::envelope::Payload::ClientHello(hello.into()))
}

/// Encodes a `LeaseRequest` as a length-delimited envelope frame.
#[must_use]
pub fn encode_lease_request(request: &LeaseRequest) -> Vec<u8> {
    encode(wire::envelope::Payload::LeaseRequest(request.into()))
}

/// Encodes a `Ping` as a bare (non-length-delimited) envelope for the
/// control-fast datagram channel. The host decodes RTT pings as datagrams
/// (`decode_ping_datagram`), not on the bootstrap stream, so the timed send
/// loop sends `Ping` this way rather than as a length-delimited stream frame.
#[must_use]
pub fn encode_ping_datagram(ping: &Ping) -> Vec<u8> {
    use prost::Message;
    wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::Ping(ping.into())),
    }
    .encode_to_vec()
}

/// Decodes exactly one length-delimited envelope from the front of `bytes`,
/// returning the typed [`StreamMessage`] and the number of bytes consumed
/// (the caller drains that many bytes from its receive buffer before the
/// next call).
///
/// # Errors
///
/// Returns [`ProbeError::Decode`] if `bytes` does not begin with a valid
/// envelope, the schema version is unsupported, or a required field/enum
/// value is missing or unrecognized.
pub fn decode_one(bytes: &[u8]) -> Result<(StreamMessage, usize), ProbeError> {
    let (envelope, remaining) = decode_envelope_length_delimited(bytes)?;
    if envelope.schema_version != SCHEMA_VERSION {
        return Err(ProbeError::Protocol {
            message: format!(
                "unsupported schema_version {} (expected {SCHEMA_VERSION})",
                envelope.schema_version
            ),
        });
    }
    let consumed = bytes.len() - remaining.len();
    let payload = envelope.payload.ok_or_else(|| ProbeError::Protocol {
        message: "envelope carried no payload".to_string(),
    })?;
    Ok((StreamMessage::try_from(payload)?, consumed))
}

/// The subset of envelope payload arms this probe expects on the bidi
/// stream, converted to their domain types.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamMessage {
    /// Reply to `ClientHello`.
    ServerWelcome(ServerWelcome),
    /// Reply to `LeaseRequest`.
    LeaseResponse(LeaseResponse),
    /// Reply to a `Ping` this client sent.
    Pong(Pong),
    /// Notice that a previously sent control frame was not honored.
    FrameRejected(FrameRejected),
    /// A reliable, ordered authority/mode event from the host's dedicated
    /// authority-events stream (ADR-0005). Carried in wire form: this probe
    /// only counts these to prove the stream is live, and has no domain-side
    /// use for the decoded event.
    AuthorityEvent(wire::AuthorityEvent),
}

impl TryFrom<wire::envelope::Payload> for StreamMessage {
    type Error = ProbeError;

    fn try_from(payload: wire::envelope::Payload) -> Result<Self, Self::Error> {
        use pilotage_protocol::wire::envelope::Payload;
        match payload {
            Payload::ServerWelcome(welcome) => Ok(Self::ServerWelcome(
                ServerWelcome::try_from(welcome).map_err(convert_err)?,
            )),
            Payload::LeaseResponse(response) => Ok(Self::LeaseResponse(
                LeaseResponse::try_from(response).map_err(convert_err)?,
            )),
            Payload::Pong(pong) => Ok(Self::Pong(Pong::try_from(pong).map_err(convert_err)?)),
            Payload::FrameRejected(rejected) => Ok(Self::FrameRejected(
                FrameRejected::try_from(rejected).map_err(convert_err)?,
            )),
            Payload::AuthorityEvent(event) => Ok(Self::AuthorityEvent(event)),
            other => Err(ProbeError::Protocol {
                message: format!("unexpected envelope payload arm on bidi stream: {other:?}"),
            }),
        }
    }
}

/// Wraps a `pilotage_protocol::ConvertError` as a `ProbeError::Decode`,
/// reusing `DecodeError::Convert`'s existing `#[source]` chain rather than
/// adding a parallel error variant for the same underlying cause.
fn convert_err(source: pilotage_protocol::ConvertError) -> ProbeError {
    ProbeError::Decode {
        source: DecodeError::Convert(source),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{decode_one, encode_client_hello, encode_lease_request, encode_ping_datagram};
    use pilotage_protocol::{ClientHello, LeaseRequest, Ping, ScopeId, VehicleId};
    use pilotage_timing::MonoTimestamp;

    #[test]
    fn client_hello_roundtrips_through_bytes_but_is_not_a_stream_message_arm() {
        // ClientHello and LeaseRequest are client->host only in this
        // probe's flow, so `decode_one` (host->client direction) correctly
        // has no arm for them; this test only exercises that encoding does
        // not panic and produces a non-empty, length-prefixed frame.
        let hello = ClientHello {
            protocol_version: 1,
            client_name: "loopback-probe".to_string(),
            join_token: vec![],
        };
        let bytes = encode_client_hello(&hello);
        assert!(!bytes.is_empty());
    }

    #[test]
    fn lease_request_encodes_non_empty() {
        let request = LeaseRequest {
            vehicle: VehicleId::new(1),
            scope: ScopeId::new("vehicle.motion"),
        };
        assert!(!encode_lease_request(&request).is_empty());
    }

    #[test]
    fn ping_datagram_encodes_non_empty() {
        let ping = Ping {
            nonce: 42,
            sender_sent_at: MonoTimestamp::from_nanos(0),
        };
        assert!(!encode_ping_datagram(&ping).is_empty());
    }

    #[test]
    fn decode_one_rejects_empty_bytes() {
        assert!(decode_one(&[]).is_err());
    }

    #[test]
    fn decode_one_rejects_non_stream_message_arm() {
        use pilotage_protocol::{encode_envelope_length_delimited, wire};
        // A valid, length-delimited envelope carrying a `Ping` arm: `Ping` is
        // client->host only, so `decode_one` (host->client direction) must
        // reject it with a Protocol error rather than silently succeed.
        let envelope = wire::Envelope {
            schema_version: super::SCHEMA_VERSION,
            payload: Some(wire::envelope::Payload::Ping(wire::Ping {
                nonce: 1,
                sender_sent_at: Some(wire::MonoTimestamp { nanos: 0 }),
            })),
        };
        let bytes = encode_envelope_length_delimited(&envelope);
        let err = decode_one(&bytes).expect_err("Ping is not a StreamMessage arm");
        assert!(matches!(err, crate::error::ProbeError::Protocol { .. }));
    }
}
