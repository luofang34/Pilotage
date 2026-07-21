//! Conversions between domain types (`control.rs`, `ids.rs`) and generated
//! wire types (`wire.rs`), plus envelope encode/decode helpers (ADR-0014).
//!
//! Domain-to-wire conversions are infallible: the domain model is always a
//! valid subset of the wire model. Wire-to-domain conversions are fallible:
//! bytes off the network may be absent, malformed, or carry an unknown enum
//! value that a newer peer defined.

use prost::Message;

use crate::control::{
    ButtonEdge, ControlPayload, LogicalAxisId, LogicalButtonId, ScopedControlFrame,
};
use crate::ids::{Generation, ScopeId, SequenceNum, SessionId, VehicleId};
use crate::wire;
use pilotage_timing::MonoTimestamp;

mod intent;

pub(crate) use intent::{
    action_from_wire as action_request_from_wire, action_to_wire as action_request_to_wire,
};

/// Errors converting a decoded wire message into its domain equivalent.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ConvertError {
    /// A `oneof` or optional field required for this conversion was absent.
    #[error("missing required field `{field}` in {message}")]
    MissingField {
        /// The wire message type that was missing a field.
        message: &'static str,
        /// The name of the missing field.
        field: &'static str,
    },
    /// A wire enum carried a numeric value with no known domain mapping,
    /// including the `UNSPECIFIED = 0` sentinel where a concrete value is
    /// required.
    #[error("unknown or unspecified enum value {value} for {enum_name}")]
    UnknownEnum {
        /// The enum type that carried the unrecognized value.
        enum_name: &'static str,
        /// The raw numeric value found on the wire.
        value: i32,
    },
    /// A wire `u32` identifier field carried a value that does not fit in
    /// the domain's narrower identifier type.
    #[error("{field} value {value} in {message} exceeds the domain identifier range")]
    IdOutOfRange {
        /// The wire message type that carried the out-of-range identifier.
        message: &'static str,
        /// The name of the out-of-range field.
        field: &'static str,
        /// The raw value found on the wire.
        value: u32,
    },
    /// A wire `f32` axis value was NaN or infinite, which cannot represent a
    /// position on the documented `[-1.0, 1.0]` axis convention.
    #[error("axis {axis_id} value {value} is not finite")]
    NonFiniteAxisValue {
        /// The logical axis identifier that carried the non-finite value.
        axis_id: u32,
        /// The raw, non-finite value found on the wire.
        value: f32,
    },
    /// A wire `f32` field of a typed control intent was NaN or infinite, which
    /// cannot represent a physical velocity, rate, orientation, or thrust.
    #[error("control intent field `{field}` is not finite")]
    NonFiniteIntentValue {
        /// The intent field that carried the non-finite value.
        field: &'static str,
    },
    /// A typed intent field was outside its documented range (a thrust must
    /// lie in `[0, 1]`), so it cannot represent a physical command.
    #[error("control intent field `{field}` value {value} is outside its documented range")]
    IntentOutOfRange {
        /// The out-of-range intent field.
        field: &'static str,
        /// The raw value found on the wire.
        value: f32,
    },
    /// An attitude intent's quaternion was not a unit rotation (within
    /// tolerance), so it cannot represent an orientation.
    #[error("attitude quaternion norm {norm} is not a unit rotation")]
    InvalidQuaternion {
        /// The quaternion's Euclidean norm as found on the wire.
        norm: f32,
    },
    /// The envelope's `schema_version` is not one this build knows how to
    /// interpret (ADR-0014).
    #[error("unsupported schema_version {found} (expected {expected})")]
    UnsupportedSchemaVersion {
        /// The schema version this build produces and accepts.
        expected: u32,
        /// The schema version found on the envelope.
        found: u32,
    },
}

/// Errors decoding raw bytes into a wire message before domain conversion.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// `prost` failed to parse the bytes as the expected message type.
    #[error("failed to decode {message}: {source}")]
    Prost {
        /// The wire message type that decoding was attempted for.
        message: &'static str,
        /// The underlying `prost` decode error.
        #[source]
        source: prost::DecodeError,
    },
    /// The bytes decoded, but conversion to the domain type failed.
    #[error(transparent)]
    Convert(#[from] ConvertError),
}

impl From<ButtonEdge> for wire::ButtonEdge {
    fn from(edge: ButtonEdge) -> Self {
        match edge {
            ButtonEdge::Pressed => wire::ButtonEdge::Pressed,
            ButtonEdge::Released => wire::ButtonEdge::Released,
        }
    }
}

impl TryFrom<wire::ButtonEdge> for ButtonEdge {
    type Error = ConvertError;

    fn try_from(edge: wire::ButtonEdge) -> Result<Self, Self::Error> {
        match edge {
            wire::ButtonEdge::Pressed => Ok(ButtonEdge::Pressed),
            wire::ButtonEdge::Released => Ok(ButtonEdge::Released),
            wire::ButtonEdge::Unspecified => Err(ConvertError::UnknownEnum {
                enum_name: "pilotage.v1.ButtonEdge",
                value: edge as i32,
            }),
        }
    }
}

fn payload_to_wire(payload: &ControlPayload) -> wire::ControlPayload {
    wire::ControlPayload {
        axes: payload
            .axes
            .iter()
            .map(|(axis, value)| wire::AxisSample {
                axis_id: u32::from(axis.as_u16()),
                value: *value,
            })
            .collect(),
        edges: payload
            .edges
            .iter()
            .map(|(button, edge)| wire::ButtonEdgeSample {
                button_id: u32::from(button.as_u16()),
                edge: wire::ButtonEdge::from(*edge) as i32,
            })
            .collect(),
    }
}

/// Converts a wire `u32` identifier into the domain's `u16` identifier
/// space, rejecting values a legitimate sender could never produce rather
/// than silently truncating them.
fn id_from_wire(
    value: u32,
    message: &'static str,
    field: &'static str,
) -> Result<u16, ConvertError> {
    u16::try_from(value).map_err(|_| ConvertError::IdOutOfRange {
        message,
        field,
        value,
    })
}

fn payload_from_wire(payload: wire::ControlPayload) -> Result<ControlPayload, ConvertError> {
    let axes = payload
        .axes
        .into_iter()
        .map(|sample| {
            let axis_id = id_from_wire(sample.axis_id, "pilotage.v1.AxisSample", "axis_id")?;
            if !sample.value.is_finite() {
                return Err(ConvertError::NonFiniteAxisValue {
                    axis_id: sample.axis_id,
                    value: sample.value,
                });
            }
            Ok((LogicalAxisId::new(axis_id), sample.value))
        })
        .collect::<Result<_, ConvertError>>()?;
    let edges = payload
        .edges
        .into_iter()
        .map(|sample| {
            let edge =
                wire::ButtonEdge::try_from(sample.edge).map_err(|_| ConvertError::UnknownEnum {
                    enum_name: "pilotage.v1.ButtonEdge",
                    value: sample.edge,
                })?;
            let button_id = id_from_wire(
                sample.button_id,
                "pilotage.v1.ButtonEdgeSample",
                "button_id",
            )?;
            Ok((LogicalButtonId::new(button_id), ButtonEdge::try_from(edge)?))
        })
        .collect::<Result<_, ConvertError>>()?;
    Ok(ControlPayload { axes, edges })
}

impl From<&ScopedControlFrame> for wire::ControlFrame {
    fn from(frame: &ScopedControlFrame) -> Self {
        wire::ControlFrame {
            session: Some(wire::SessionId {
                value: frame.session.as_u64(),
            }),
            vehicle: Some(wire::VehicleId {
                value: frame.vehicle.as_u64(),
            }),
            scope: Some(wire::ScopeId {
                value: frame.scope.as_str().to_owned(),
            }),
            generation: Some(wire::Generation {
                value: frame.generation.as_u64(),
            }),
            sequence: Some(wire::SequenceNum {
                value: frame.sequence.as_u32(),
            }),
            sampled_at: Some(wire::MonoTimestamp {
                nanos: frame.sampled_at.as_nanos(),
            }),
            profile_revision: frame.profile_revision,
            activation_revision: frame.activation_revision,
            // A typed-only frame omits the payload field entirely; presence
            // on the wire is decided by content, never by an empty message.
            payload: frame
                .carries_payload()
                .then(|| payload_to_wire(&frame.payload)),
            intent: frame.intent.as_ref().map(intent::intent_to_wire),
            actions: frame
                .actions
                .iter()
                .map(|action| intent::action_to_wire(*action))
                .collect(),
        }
    }
}

impl TryFrom<wire::ControlFrame> for ScopedControlFrame {
    type Error = ConvertError;

    fn try_from(frame: wire::ControlFrame) -> Result<Self, Self::Error> {
        let missing = |field: &'static str| ConvertError::MissingField {
            message: "pilotage.v1.ControlFrame",
            field,
        };
        let session = frame.session.ok_or_else(|| missing("session"))?;
        let vehicle = frame.vehicle.ok_or_else(|| missing("vehicle"))?;
        let scope = frame.scope.ok_or_else(|| missing("scope"))?;
        let generation = frame.generation.ok_or_else(|| missing("generation"))?;
        let sequence = frame.sequence.ok_or_else(|| missing("sequence"))?;
        let sampled_at = frame.sampled_at.ok_or_else(|| missing("sampled_at"))?;
        // A typed-only frame legitimately carries no payload field; an absent
        // payload decodes as the empty payload, and the session host's
        // exactly-one-representation rule judges the CONTENT.
        let payload = frame
            .payload
            .map(payload_from_wire)
            .transpose()?
            .unwrap_or_default();
        let control_intent = frame.intent.map(intent::intent_from_wire).transpose()?;
        let actions = frame
            .actions
            .into_iter()
            .map(intent::action_from_wire)
            .collect::<Result<_, ConvertError>>()?;

        Ok(ScopedControlFrame {
            session: SessionId::new(session.value),
            vehicle: VehicleId::new(vehicle.value),
            scope: ScopeId::new(scope.value),
            generation: Generation::new(generation.value),
            sequence: SequenceNum::new(sequence.value),
            sampled_at: MonoTimestamp::from_nanos(sampled_at.nanos),
            profile_revision: frame.profile_revision,
            activation_revision: frame.activation_revision,
            payload,
            intent: control_intent,
            actions,
        })
    }
}

/// The schema version this build of `pilotage-protocol` produces on
/// [`encode_control_frame_envelope`]. Receivers decide independently
/// whether they accept it (ADR-0014).
pub const SCHEMA_VERSION: u32 = 1;

/// Encodes a `ControlFrame` payload into a versioned [`wire::Envelope`] and
/// serializes it to bytes, suitable for a single WebTransport datagram.
#[must_use]
pub fn encode_control_frame_envelope(frame: &ScopedControlFrame) -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ControlFrame(frame.into())),
    };
    envelope.encode_to_vec()
}

/// Decodes bytes as a [`wire::Envelope`] and, if its payload is a
/// `ControlFrame`, converts it to the domain [`ScopedControlFrame`].
///
/// # Errors
///
/// Returns [`DecodeError::Prost`] if the bytes are not a valid envelope, and
/// [`DecodeError::Convert`] if the envelope's `schema_version` is not
/// [`SCHEMA_VERSION`] (ADR-0014), or if the envelope decodes but is missing
/// a required field, carries an unrecognized enum value, or has a payload
/// arm other than `ControlFrame`.
pub fn decode_control_frame_envelope(bytes: &[u8]) -> Result<ScopedControlFrame, DecodeError> {
    let envelope = wire::Envelope::decode(bytes).map_err(|source| DecodeError::Prost {
        message: "pilotage.v1.Envelope",
        source,
    })?;
    if envelope.schema_version != SCHEMA_VERSION {
        return Err(ConvertError::UnsupportedSchemaVersion {
            expected: SCHEMA_VERSION,
            found: envelope.schema_version,
        }
        .into());
    }
    let payload = envelope.payload.ok_or(ConvertError::MissingField {
        message: "pilotage.v1.Envelope",
        field: "payload",
    })?;
    match payload {
        wire::envelope::Payload::ControlFrame(frame) => Ok(ScopedControlFrame::try_from(frame)?),
        _ => Err(ConvertError::MissingField {
            message: "pilotage.v1.Envelope",
            field: "control_frame",
        }
        .into()),
    }
}

/// Encodes an [`wire::Envelope`] with a length-delimited prefix, for framing
/// on a reliable stream where multiple envelopes are concatenated
/// (ADR-0014).
#[must_use]
pub fn encode_envelope_length_delimited(envelope: &wire::Envelope) -> Vec<u8> {
    let mut buf = Vec::with_capacity(envelope.encoded_len() + 10);
    #[allow(clippy::expect_used)]
    envelope
        .encode_length_delimited(&mut buf)
        .expect("encoding into a growable Vec<u8> cannot fail");
    buf
}

/// Decodes exactly one length-delimited [`wire::Envelope`] from the front of
/// `bytes`, returning the decoded envelope and the remaining unconsumed
/// bytes for the next frame.
///
/// # Errors
///
/// Returns [`DecodeError::Prost`] if `bytes` does not begin with a valid
/// length-delimited envelope frame.
pub fn decode_envelope_length_delimited(
    bytes: &[u8],
) -> Result<(wire::Envelope, &[u8]), DecodeError> {
    let mut cursor = bytes;
    let start_len = cursor.len();
    let envelope = wire::Envelope::decode_length_delimited(&mut cursor).map_err(|source| {
        DecodeError::Prost {
            message: "pilotage.v1.Envelope",
            source,
        }
    })?;
    let consumed = start_len - cursor.len();
    Ok((envelope, &bytes[consumed..]))
}

#[cfg(test)]
mod tests;
