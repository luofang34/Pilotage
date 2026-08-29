//! The physical control family and envelope that give a stimulus its meaning.
//!
//! A stimulus waveform carries normalized values only. The control family, the
//! mapping rule, and the versioned envelope state which physical command those
//! values request. A runtime that cannot command the family refuses the
//! scenario before it touches a simulator.

mod combination;
mod error;

use serde::{Deserialize, Serialize};
use sha2::{Digest as ShaDigest, Sha256};

use super::ControlChannel;
use crate::{
    BackendCapability, CodecError, Digest, MAX_STIMULUS_ENVELOPE_BYTES, MAX_TEXT_BYTES,
    ValidationError, canonical, validation::text,
};

pub use error::StimulusError;

const ENVELOPE_DIGEST_DOMAIN: &[u8] = b"pilotage.trial.stimulus-envelope.v1\0";

/// The physical control family that one stimulus commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlFamily {
    /// The stimulus commands an operator velocity input.
    OperatorVelocity,
    /// The stimulus commands a direct attitude and thrust setpoint.
    DirectAttitudeThrust,
}

/// The rule that turns a normalized stimulus value into a physical command.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StimulusMapping {
    /// The envelope bounds the output of the candidate response curve.
    ///
    /// The scenario freezes the normalized input and the physical bound. The
    /// run binds the candidate feel profile that shapes the values between the
    /// endpoints.
    CandidateBoundCurve,
    /// The envelope resolves the output with a two-segment affine map.
    AffineExact,
}

/// The physical unit of one stimulus envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicalUnit {
    /// A linear speed in meters per second.
    MetersPerSecond,
    /// An angular rate in radians per second.
    RadiansPerSecond,
    /// An angle in radians.
    Radians,
    /// A collective force normalized against the vehicle hover force.
    NormalizedCollectiveForce,
}

/// The value that the envelope endpoints measure from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceRule {
    /// The endpoints measure from zero.
    Zero,
    /// The endpoints measure from the effective setpoint at stimulus entry.
    EffectiveSetpointAtEntry,
    /// The endpoints measure from the identified hover trim of the vehicle.
    IdentifiedHoverTrim,
}

/// A versioned physical envelope for one stimulus.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StimulusEnvelope {
    /// The stable envelope identifier.
    pub id: String,
    /// The envelope revision number.
    pub revision: u32,
    /// The physical unit of the three envelope values.
    pub unit: PhysicalUnit,
    /// The value that the endpoints measure from.
    pub reference: ReferenceRule,
    /// The physical value at normalized minus one.
    pub negative_endpoint: f64,
    /// The physical value at normalized zero.
    pub neutral: f64,
    /// The physical value at normalized plus one.
    pub positive_endpoint: f64,
}

impl ControlFamily {
    /// Gets the stable control-family name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OperatorVelocity => "operator_velocity",
            Self::DirectAttitudeThrust => "direct_attitude_thrust",
        }
    }

    /// Gets the backend capability that the family needs.
    #[must_use]
    pub const fn capability(self) -> BackendCapability {
        match self {
            Self::OperatorVelocity => BackendCapability::OperatorVelocityControl,
            Self::DirectAttitudeThrust => BackendCapability::DirectAttitudeThrustControl,
        }
    }

    /// Gets the one mapping rule that the family permits.
    #[must_use]
    pub const fn mapping(self) -> StimulusMapping {
        match self {
            Self::OperatorVelocity => StimulusMapping::CandidateBoundCurve,
            Self::DirectAttitudeThrust => StimulusMapping::AffineExact,
        }
    }
}

impl StimulusMapping {
    /// Gets the stable mapping-rule name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CandidateBoundCurve => "candidate_bound_curve",
            Self::AffineExact => "affine_exact",
        }
    }

    /// Resolves the exact physical command for one normalized value.
    ///
    /// # Errors
    ///
    /// Returns an error when the mapping needs a candidate feel profile, when
    /// the envelope is invalid, or when the normalized value is outside
    /// minus one through plus one.
    pub fn resolve_exact(
        self,
        envelope: &StimulusEnvelope,
        normalized: f64,
    ) -> Result<f64, StimulusError> {
        match self {
            Self::AffineExact => envelope.map_affine(normalized),
            Self::CandidateBoundCurve => Err(StimulusError::InexactMapping {
                mapping: self.as_str(),
            }),
        }
    }
}

impl PhysicalUnit {
    /// Gets the stable unit name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MetersPerSecond => "meters_per_second",
            Self::RadiansPerSecond => "radians_per_second",
            Self::Radians => "radians",
            Self::NormalizedCollectiveForce => "normalized_collective_force",
        }
    }
}

impl ReferenceRule {
    /// Gets the stable reference-rule name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::EffectiveSetpointAtEntry => "effective_setpoint_at_entry",
            Self::IdentifiedHoverTrim => "identified_hover_trim",
        }
    }
}

impl StimulusEnvelope {
    /// Calculates the digest of the canonical envelope bytes.
    ///
    /// The digest covers every envelope field, so a changed identifier,
    /// revision, unit, reference rule, or endpoint changes the value.
    ///
    /// # Errors
    ///
    /// Returns an error when the envelope cannot be encoded.
    pub fn canonical_digest(&self) -> Result<Digest, CodecError> {
        let bytes = canonical::encode("stimulus envelope", self, MAX_STIMULUS_ENVELOPE_BYTES)?;
        let mut hasher = Sha256::new();
        hasher.update(ENVELOPE_DIGEST_DOMAIN);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        Ok(Digest::from_bytes(hasher.finalize().into()))
    }

    /// Maps one normalized value with the two-segment affine rule.
    ///
    /// The negative segment covers minus one through zero and the positive
    /// segment covers zero through plus one. Both segments meet at the neutral
    /// value.
    ///
    /// # Errors
    ///
    /// Returns an error when the envelope values are invalid or when the
    /// normalized value is outside minus one through plus one.
    pub fn map_affine(&self, normalized: f64) -> Result<f64, StimulusError> {
        self.validate_values()?;
        if !(-1.0..=1.0).contains(&normalized) {
            return Err(StimulusError::NormalizedOutOfRange { value: normalized });
        }
        let span = if normalized < 0.0 {
            self.neutral - self.negative_endpoint
        } else {
            self.positive_endpoint - self.neutral
        };
        Ok(self.neutral + normalized * span)
    }

    /// Checks the envelope values without a family or channel.
    ///
    /// # Errors
    ///
    /// Returns an error for a value that is not finite, for reversed
    /// endpoints, for a zero physical span, and for a neutral value that the
    /// endpoints do not contain.
    pub fn validate_values(&self) -> Result<(), StimulusError> {
        for (name, value) in [
            ("negative_endpoint", self.negative_endpoint),
            ("neutral", self.neutral),
            ("positive_endpoint", self.positive_endpoint),
        ] {
            if !value.is_finite() {
                return Err(StimulusError::NonFiniteValue { name });
            }
        }
        if self.negative_endpoint > self.positive_endpoint {
            return Err(StimulusError::ReversedEndpoints {
                negative: self.negative_endpoint,
                positive: self.positive_endpoint,
            });
        }
        if self.negative_endpoint == self.positive_endpoint {
            return Err(StimulusError::ZeroSpan {
                value: self.negative_endpoint,
            });
        }
        if self.neutral <= self.negative_endpoint || self.neutral >= self.positive_endpoint {
            return Err(StimulusError::NeutralOutsideEndpoints {
                neutral: self.neutral,
                negative: self.negative_endpoint,
                positive: self.positive_endpoint,
            });
        }
        Ok(())
    }
}

/// Checks the complete physical meaning of one stimulus.
pub(super) fn validate(
    field: &str,
    family: ControlFamily,
    channel: ControlChannel,
    mapping: StimulusMapping,
    envelope: &StimulusEnvelope,
) -> Result<(), ValidationError> {
    text(
        &format!("{field}.envelope.id"),
        &envelope.id,
        MAX_TEXT_BYTES,
    )?;
    let wrap = |source| ValidationError::InvalidStimulus {
        field: field.to_owned(),
        source,
    };
    envelope.validate_values().map_err(wrap)?;
    combination::validate(family, channel, mapping, envelope).map_err(wrap)
}

#[cfg(test)]
mod tests;
