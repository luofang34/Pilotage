//! What the agent can fly at this moment.
//!
//! The envelope goes to the model with each request, so a model picks from
//! what is permitted. It also checks each reply, so a directive outside it is
//! refused and is not flown.

use serde::{Deserialize, Serialize};

use crate::directive::DirectiveKind;

/// The permitted values of one number slot.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NumberRange {
    /// The smallest permitted value.
    pub min: f64,
    /// The largest permitted value.
    pub max: f64,
}

impl NumberRange {
    /// True when `value` is a finite number in the range.
    #[must_use]
    pub fn admits(&self, value: f64) -> bool {
        value.is_finite() && value >= self.min && value <= self.max
    }
}

/// The directive kinds, names and number ranges that are permitted now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlightEnvelope {
    /// The directive kinds that the agent can fly.
    pub kinds: Vec<DirectiveKind>,
    /// The fix names that a directive can refer to.
    pub fixes: Vec<String>,
    /// The procedure names that a directive can refer to.
    pub procedures: Vec<String>,
    /// The permitted height in metres above the launch point.
    pub height_m: NumberRange,
    /// The permitted ground speed in metres per second.
    pub speed_mps: NumberRange,
}

impl FlightEnvelope {
    /// True when the agent can fly `kind` now.
    #[must_use]
    pub fn offers(&self, kind: DirectiveKind) -> bool {
        self.kinds.contains(&kind)
    }
}
