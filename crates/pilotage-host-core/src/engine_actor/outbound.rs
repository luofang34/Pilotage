//! Maps engine-outbound envelopes onto their connection-task arms.

use crate::connection::{DatagramClass, ToConnection};
use crate::wire_codec::{encode_envelope_message, encode_pong_datagram};

/// Encodes an [`OutboundMessage`] on its [`ToConnection`] arm (ADR-0005); `Pong`
/// is a datagram; `Authority` uses the dedicated event stream so bootstrap
/// traffic cannot block it; every other arm uses the bootstrap stream.
///
/// [`OutboundMessage`]: pilotage_session::OutboundMessage
pub fn to_connection_message(envelope: &pilotage_session::OutboundMessage) -> ToConnection {
    match envelope {
        pilotage_session::OutboundMessage::Pong(pong) => ToConnection::Datagram {
            class: DatagramClass::Pong,
            bytes: encode_pong_datagram(pong),
        },
        // The link-loss-cleared notice is a reliable broadcast; it rides the
        // dedicated authority-events stream alongside authority events so a
        // bulk/bootstrap transfer cannot head-of-line-block a recovery ack.
        pilotage_session::OutboundMessage::Authority(_)
        | pilotage_session::OutboundMessage::LinkLossCleared(_) => {
            ToConnection::AuthorityMessage(encode_envelope_message(envelope))
        }
        pilotage_session::OutboundMessage::Capabilities(_)
        | pilotage_session::OutboundMessage::Welcome(_)
        | pilotage_session::OutboundMessage::LeaseResponse(_)
        | pilotage_session::OutboundMessage::LeaseReleased(_)
        | pilotage_session::OutboundMessage::ControlActionResult(_)
        | pilotage_session::OutboundMessage::FrameRejected(_) => ToConnection::BootstrapMessage {
            bytes: encode_envelope_message(envelope),
            opens_media: matches!(envelope, pilotage_session::OutboundMessage::Welcome(_)),
        },
    }
}
