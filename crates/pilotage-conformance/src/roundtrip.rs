//! Wire round-trip conformance for the two message families the increment-0
//! session exchanges (ADR-0012, ADR-0014).
//!
//! Control frames must survive `domain -> wire -> domain` unchanged: this is
//! the client-core/test-host exchange increment 0 accepts. Authority effects
//! are the persisted audit trail (ADR-0012); they must survive
//! `effect -> wire -> bytes -> wire` unchanged, exercising both the
//! `pilotage-authority` effect->wire conversion and the `prost`
//! encode/decode of the resulting `pilotage.v1.AuthorityEvent`.

use pilotage_authority::AuthorityEffect;
use pilotage_protocol::wire as proto;
use pilotage_protocol::{
    DecodeError, ScopedControlFrame, decode_control_frame_envelope, encode_control_frame_envelope,
};
use prost::Message;

/// An error establishing a wire round-trip for a session message.
#[derive(Debug, thiserror::Error)]
pub enum RoundTripError {
    /// A control-frame envelope failed to decode back to its domain form.
    #[error("control frame envelope failed to decode: {source}")]
    ControlDecode {
        /// The underlying protocol decode error.
        #[source]
        source: DecodeError,
    },
    /// A decoded control frame did not equal the original domain frame.
    #[error("control frame did not round-trip equal for sequence {sequence}")]
    ControlMismatch {
        /// Sequence number of the frame that failed to round-trip.
        sequence: u32,
    },
    /// A re-encoded authority event failed to decode as a wire event.
    #[error("authority event failed to decode: {source}")]
    AuthorityDecode {
        /// The underlying `prost` decode error.
        #[source]
        source: prost::DecodeError,
    },
    /// A decoded authority event did not equal its direct wire conversion.
    #[error("authority event did not round-trip equal for {kind:?}")]
    AuthorityMismatch {
        /// Wire event kind of the effect that failed to round-trip.
        kind: pilotage_authority::WireEventKind,
    },
}

/// Encodes `frame` into a versioned envelope, decodes it, and returns the
/// decoded frame; the caller asserts equality, or this returns a
/// [`RoundTripError`] on decode failure or inequality.
///
/// # Errors
///
/// Returns [`RoundTripError::ControlDecode`] if the encoded envelope does not
/// decode, and [`RoundTripError::ControlMismatch`] if the decoded frame
/// differs from `frame`.
pub fn control_frame_roundtrips(frame: &ScopedControlFrame) -> Result<(), RoundTripError> {
    let bytes = encode_control_frame_envelope(frame);
    let decoded = decode_control_frame_envelope(&bytes)
        .map_err(|source| RoundTripError::ControlDecode { source })?;
    if &decoded == frame {
        Ok(())
    } else {
        Err(RoundTripError::ControlMismatch {
            sequence: frame.sequence.as_u32(),
        })
    }
}

/// Converts `effect` to its wire authority event, serializes it with `prost`,
/// decodes the bytes back to a wire event, and checks the decoded event
/// equals the direct conversion.
///
/// The authority crate ships only the `effect -> wire` direction (wire events
/// are the persisted audit form, never decoded back into engine effects), so
/// the round-trip asserted here is the wire event's own serialization
/// stability: `AuthorityEvent -> bytes -> AuthorityEvent` is the identity.
///
/// # Errors
///
/// Returns [`RoundTripError::AuthorityDecode`] if the serialized event does
/// not decode, and [`RoundTripError::AuthorityMismatch`] if the decoded event
/// differs from the direct conversion.
pub fn authority_event_roundtrips(effect: &AuthorityEffect) -> Result<(), RoundTripError> {
    let event = proto::AuthorityEvent::from(effect);
    let bytes = event.encode_to_vec();
    let decoded = proto::AuthorityEvent::decode(bytes.as_slice())
        .map_err(|source| RoundTripError::AuthorityDecode { source })?;
    if decoded == event {
        Ok(())
    } else {
        Err(RoundTripError::AuthorityMismatch {
            kind: effect.wire_event_kind(),
        })
    }
}
