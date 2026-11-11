//! The engine's outbound envelope encoding, from the session host's wire
//! codec: the engine answers its in-core clients with the same bytes a
//! remote client reads.

use pilotage_protocol::wire;
use pilotage_protocol::{SCHEMA_VERSION, encode_envelope_length_delimited};
use pilotage_session::OutboundMessage;
use prost::Message;

/// Encodes an [`OutboundMessage`] as a length-delimited envelope. Used for
/// both the bootstrap stream (`Welcome`/`LeaseResponse`) and the dedicated
/// session-events stream (`Authority`) — the framing is identical, only
/// the destination stream differs (ADR-0005).
#[must_use]
pub fn encode_envelope_message(message: &OutboundMessage) -> Vec<u8> {
    let payload = match message {
        OutboundMessage::Capabilities(capabilities) => {
            wire::envelope::Payload::HostCapabilities(capabilities.clone())
        }
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
        OutboundMessage::FrameRejected(rejected) => {
            wire::envelope::Payload::FrameRejected(rejected.into())
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
