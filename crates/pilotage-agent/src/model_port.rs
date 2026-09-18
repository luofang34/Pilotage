//! The model port: the only boundary between Pilotage and a model.
//!
//! A model adapter is a separate process. It reads one request line and writes
//! one reply line, each a JSON object. The first line that it writes is its
//! declaration. Pilotage links no model and selects no model.
//!
//! A request carries the newest operator message, the flight envelope, a
//! legend for the names, and zero or more frames. It carries no vehicle state
//! and no earlier messages. A small model reads both as instructions.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::capability::FlightEnvelope;
use crate::directive::{Directive, DirectiveKind, HoldPoint};

/// How an image maps the scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    /// A normal camera image.
    Rectilinear,
    /// A full panorama in one image, longitude by latitude.
    Equirectangular,
}

/// One image for the model. A panorama is one equirectangular frame, or a set
/// of rectilinear frames that have different source identities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    /// The identity of the video source, as the session catalog gives it.
    pub source_id: String,
    /// What the source shows, such as `forward`, `down` or `gimbal`.
    pub role: String,
    /// The capture time in nanoseconds on the source clock.
    pub captured_at_ns: u64,
    /// The media type of the data, such as `image/jpeg`.
    pub media_type: String,
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// How the image maps the scene.
    pub projection: Projection,
    /// The image bytes in base64.
    pub data_base64: String,
}

/// The frames that an adapter accepts.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FrameSupport {
    /// The largest number of frames in one request. Zero means text only.
    pub max_frames: u8,
    /// The projections that the adapter can read.
    #[serde(default)]
    pub projections: Vec<Projection>,
}

/// The first line from an adapter: what it is and what it supports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterDeclaration {
    /// True when the model is loaded and the adapter can take requests.
    pub ready: bool,
    /// The adapter name and version.
    pub adapter: String,
    /// The model identity, as exact as the adapter knows it.
    pub model: String,
    /// The directive kinds that the adapter can give.
    pub kinds: Vec<DirectiveKind>,
    /// The frames that the adapter accepts.
    #[serde(default)]
    pub frames: FrameSupport,
}

/// One request to a model adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRequest {
    /// The newest operator message, word for word.
    pub message: String,
    /// What the agent can fly now.
    pub envelope: FlightEnvelope,
    /// A short text that tells where each named fix is.
    pub legend: String,
    /// Images for the model. Empty for a text request.
    #[serde(default)]
    pub frames: Vec<Frame>,
}

/// The probability of each slot, when the model has one. The key is the slot
/// name, and `kind` is the key for the directive kind.
pub type SlotProbabilities = BTreeMap<String, f64>;

/// One reply from a model adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelReply {
    /// The directive that the model read from the message.
    pub directive: Directive,
    /// The probability of each slot.
    #[serde(default)]
    pub probabilities: SlotProbabilities,
    /// The inference time that the adapter measured, in milliseconds.
    #[serde(default)]
    pub model_ms: f64,
}

/// Why a reply is not flown.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Refusal {
    /// The envelope does not offer this directive kind now.
    #[error("the directive kind {0:?} is not offered now")]
    KindNotOffered(DirectiveKind),
    /// The directive names a fix that the envelope does not have.
    #[error("the fix {0} is not known")]
    UnknownFix(String),
    /// The directive names a procedure that the envelope does not have.
    #[error("the procedure {0} is not known")]
    UnknownProcedure(String),
    /// A name or a number of the directive is not in the operator message.
    #[error("the message does not say the {slot} value {value}")]
    NotInMessage {
        /// The slot name.
        slot: &'static str,
        /// The value that the message does not have.
        value: String,
    },
    /// A number slot is outside its permitted range.
    #[error("the {slot} value {value} is outside the permitted range")]
    OutOfRange {
        /// The slot name.
        slot: &'static str,
        /// The refused value.
        value: f64,
    },
    /// The directive changes nothing in the present phase, so the vehicle
    /// keeps its directive. A silent no-op would read as a flown directive.
    #[error("the directive {kind:?} has no effect in the phase {phase:?}")]
    NoEffect {
        /// The directive kind.
        kind: DirectiveKind,
        /// The executor phase.
        phase: crate::executor::Phase,
    },
}

/// Checks a reply against the envelope of its request. The agent flies only a
/// directive that passes. An out-of-range number is refused and is not
/// clamped, because a clamped number is a different instruction.
pub fn check_reply(envelope: &FlightEnvelope, reply: &ModelReply) -> Result<Directive, Refusal> {
    let directive = &reply.directive;
    if directive.kind() != DirectiveKind::Unable && !envelope.offers(directive.kind()) {
        return Err(Refusal::KindNotOffered(directive.kind()));
    }
    let known_fix = |fix: &String| {
        if envelope.fixes.contains(fix) {
            Ok(())
        } else {
            Err(Refusal::UnknownFix(fix.clone()))
        }
    };
    let out_of_range = |slot, value| Err(Refusal::OutOfRange { slot, value });
    match directive {
        Directive::DirectTo { fix, .. }
        | Directive::Hold {
            point: HoldPoint::Fix(fix),
        } => known_fix(fix)?,
        Directive::JoinProcedure { procedure } if !envelope.procedures.contains(procedure) => {
            return Err(Refusal::UnknownProcedure(procedure.clone()));
        }
        Directive::Heading { degrees, .. } if *degrees >= 360 => {
            return out_of_range("degrees", f64::from(*degrees));
        }
        Directive::Altitude { height_m } if !envelope.height_m.admits(*height_m) => {
            return out_of_range("height_m", *height_m);
        }
        Directive::Speed { speed_mps } if !envelope.speed_mps.admits(*speed_mps) => {
            return out_of_range("speed_mps", *speed_mps);
        }
        _ => {}
    }
    Ok(directive.clone())
}

#[cfg(test)]
mod tests;
