//! Exact physical targets, and the prepared command that carries one.
//!
//! A prepared command is complete before any datagram can leave the
//! process. The transport re-derives it at enactment and refuses a target
//! that changed, so a substituted target, channel, or family is caught
//! while the command is still a value in memory.

use flight_tune::{
    ControlChannel, ControlFamily, Digest, PhysicalUnit, ReferenceRule, StimulusEnvelope,
    StimulusMapping,
};
use serde::{Deserialize, Serialize};

use super::error::DirectTransportError;
use super::port::DirectSetpoint;

/// What one direct command is for.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectCommandPurpose {
    /// One command in the block that establishes the frozen baseline.
    Baseline,
    /// The scored exact step.
    Step,
    /// The family-aware release back to the frozen baseline.
    Release,
}

impl DirectCommandPurpose {
    /// The stable name of this purpose.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Step => "step",
            Self::Release => "release",
        }
    }
}

/// One requested exact direct step.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectStepRequest {
    /// The physical control family that the stimulus commands.
    pub family: ControlFamily,
    /// The control channel that the step moves.
    pub channel: ControlChannel,
    /// The rule that resolves a normalized value to a physical command.
    pub mapping: StimulusMapping,
    /// The frozen physical envelope of the normalized range.
    pub envelope: StimulusEnvelope,
    /// The normalized stimulus value.
    pub normalized: f64,
}

/// One direct command, complete before any datagram leaves the process.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedDirectCommand {
    pub(super) purpose: DirectCommandPurpose,
    pub(super) stimulus: DirectStepRequest,
    pub(super) envelope_digest: Digest,
    pub(super) baseline: DirectSetpoint,
    pub(super) requested: DirectSetpoint,
    pub(super) run_intent_digest: Digest,
    pub(super) transport_identity_digest: Digest,
}

impl PreparedDirectCommand {
    /// What this command is for.
    #[must_use]
    pub const fn purpose(&self) -> DirectCommandPurpose {
        self.purpose
    }

    /// The stimulus that this command resolves.
    #[must_use]
    pub const fn stimulus(&self) -> &DirectStepRequest {
        &self.stimulus
    }

    /// The control channel that this command moves.
    #[must_use]
    pub const fn channel(&self) -> ControlChannel {
        self.stimulus.channel
    }

    /// The frozen stimulus envelope of this command.
    #[must_use]
    pub const fn envelope_digest(&self) -> Digest {
        self.envelope_digest
    }

    /// The physical target that the command must transmit unchanged.
    #[must_use]
    pub const fn requested(&self) -> DirectSetpoint {
        self.requested
    }

    /// The frozen direct baseline that this command was built from.
    #[must_use]
    pub const fn baseline(&self) -> DirectSetpoint {
        self.baseline
    }

    /// The run intent that this command binds to.
    #[must_use]
    pub const fn run_intent_digest(&self) -> Digest {
        self.run_intent_digest
    }

    /// Replaces the physical target, for a tamper test.
    ///
    /// The transport re-derives every prepared target before it enacts it,
    /// so this exists to prove that the re-derivation refuses a changed
    /// target while the command is still a value in memory.
    #[must_use]
    pub const fn with_requested_for_test(mut self, requested: DirectSetpoint) -> Self {
        self.requested = requested;
        self
    }

    /// Replaces the stimulus, for a tamper test.
    ///
    /// A substituted family, channel, envelope, or normalized value has to
    /// be caught by the same re-derivation.
    #[must_use]
    pub fn with_stimulus_for_test(mut self, stimulus: DirectStepRequest) -> Self {
        self.stimulus = stimulus;
        self
    }
}

/// Applies one exact physical offset to a frozen direct baseline.
///
/// The envelope resolves an offset from its reference rule, so every
/// channel other than the commanded one keeps its frozen baseline value
/// bit for bit.
///
/// # Errors
///
/// Returns [`DirectTransportError`] when the family is not the direct
/// attitude and thrust family, when the mapping is not exact, when the
/// envelope physics do not match the channel, or when the envelope refuses
/// the normalized value.
pub fn resolve_exact_target(
    request: &DirectStepRequest,
    baseline: DirectSetpoint,
) -> Result<DirectSetpoint, DirectTransportError> {
    require_direct_family(request.family)?;
    if request.mapping != StimulusMapping::AffineExact {
        return Err(DirectTransportError::InexactMapping);
    }
    require_channel_physics(request.channel, &request.envelope)?;
    let offset = request
        .mapping
        .resolve_exact(&request.envelope, request.normalized)
        .map_err(|source| DirectTransportError::Envelope { source })?;
    if !offset.is_finite() {
        return Err(DirectTransportError::InvalidValue {
            field: "physical offset",
        });
    }
    let target = match request.channel {
        ControlChannel::Roll => DirectSetpoint {
            roll_rad: baseline.roll_rad + offset,
            ..baseline
        },
        ControlChannel::Pitch => DirectSetpoint {
            pitch_rad: baseline.pitch_rad + offset,
            ..baseline
        },
        ControlChannel::Yaw => DirectSetpoint {
            yaw_rad: baseline.yaw_rad + offset,
            ..baseline
        },
        // The frozen baseline collective IS the identified hover trim, so
        // the vertical offset measures from the same value its reference
        // rule names.
        ControlChannel::Vertical => DirectSetpoint {
            collective_force: baseline.collective_force + offset,
            ..baseline
        },
    };
    if !target.is_finite() {
        return Err(DirectTransportError::InvalidValue {
            field: "physical target",
        });
    }
    Ok(target)
}

/// Rejects a control family that the direct transport does not carry.
///
/// # Errors
///
/// Returns [`DirectTransportError`] for the operator velocity family,
/// which keeps its own shaped path.
pub fn require_direct_family(family: ControlFamily) -> Result<(), DirectTransportError> {
    match family {
        ControlFamily::DirectAttitudeThrust => Ok(()),
        ControlFamily::OperatorVelocity => Err(DirectTransportError::UnsupportedFamily {
            family: family.as_str().to_owned(),
        }),
    }
}

/// The digest of one frozen stimulus envelope.
///
/// # Errors
///
/// Returns [`DirectTransportError`] when the envelope cannot be encoded.
pub fn envelope_digest(envelope: &StimulusEnvelope) -> Result<Digest, DirectTransportError> {
    // The mission document and the tuning harness each carry their own
    // digest type over the same 32 bytes, so the value crosses by bytes.
    let canonical =
        envelope
            .canonical_digest()
            .map_err(|error| DirectTransportError::IncompleteIdentity {
                detail: format!("the stimulus envelope has no canonical digest: {error}"),
            })?;
    Ok(Digest::from_bytes(*canonical.as_bytes()))
}

/// The envelope physics that each direct channel must declare.
///
/// The authored document is validated upstream. The transport checks the
/// same rule again because it is the last place that can refuse a stimulus
/// before a datagram commands the vehicle.
fn require_channel_physics(
    channel: ControlChannel,
    envelope: &StimulusEnvelope,
) -> Result<(), DirectTransportError> {
    let (unit, reference) = match channel {
        ControlChannel::Roll | ControlChannel::Pitch | ControlChannel::Yaw => (
            PhysicalUnit::Radians,
            ReferenceRule::EffectiveSetpointAtEntry,
        ),
        ControlChannel::Vertical => (
            PhysicalUnit::NormalizedCollectiveForce,
            ReferenceRule::IdentifiedHoverTrim,
        ),
    };
    if envelope.unit != unit {
        return Err(DirectTransportError::EnvelopePhysics {
            channel: channel_name(channel).to_owned(),
            detail: "another physical unit".to_owned(),
        });
    }
    if envelope.reference != reference {
        return Err(DirectTransportError::EnvelopePhysics {
            channel: channel_name(channel).to_owned(),
            detail: "another reference rule".to_owned(),
        });
    }
    Ok(())
}

pub(super) const fn channel_name(channel: ControlChannel) -> &'static str {
    match channel {
        ControlChannel::Roll => "roll",
        ControlChannel::Pitch => "pitch",
        ControlChannel::Yaw => "yaw",
        ControlChannel::Vertical => "vertical",
    }
}
