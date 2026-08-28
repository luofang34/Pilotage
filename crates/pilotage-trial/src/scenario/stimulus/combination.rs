//! The permitted control family, channel, unit, and reference combinations.

use super::{
    ControlFamily, PhysicalUnit, ReferenceRule, StimulusEnvelope, StimulusError, StimulusMapping,
};
use crate::ControlChannel;

/// Checks one stimulus against the permitted physical combinations.
pub(super) fn validate(
    family: ControlFamily,
    channel: ControlChannel,
    mapping: StimulusMapping,
    envelope: &StimulusEnvelope,
) -> Result<(), StimulusError> {
    let (unit, reference) = required_physics(family, channel);
    if envelope.unit != unit {
        return Err(StimulusError::UnitMismatch {
            family: family.as_str(),
            channel: channel_name(channel),
            expected: unit.as_str(),
            actual: envelope.unit.as_str(),
        });
    }
    if envelope.reference != reference {
        return Err(StimulusError::ReferenceMismatch {
            family: family.as_str(),
            channel: channel_name(channel),
            expected: reference.as_str(),
            actual: envelope.reference.as_str(),
        });
    }
    let required = family.mapping();
    if mapping != required {
        return Err(StimulusError::MappingMismatch {
            family: family.as_str(),
            expected: required.as_str(),
            actual: mapping.as_str(),
        });
    }
    Ok(())
}

/// Gets the one unit and reference rule that a family and channel permit.
///
/// The match is exhaustive over both enums, so a combination that this table
/// does not name cannot exist.
const fn required_physics(
    family: ControlFamily,
    channel: ControlChannel,
) -> (PhysicalUnit, ReferenceRule) {
    match (family, channel) {
        (
            ControlFamily::OperatorVelocity,
            ControlChannel::Roll | ControlChannel::Pitch | ControlChannel::Vertical,
        ) => (PhysicalUnit::MetersPerSecond, ReferenceRule::Zero),
        (ControlFamily::OperatorVelocity, ControlChannel::Yaw) => {
            (PhysicalUnit::RadiansPerSecond, ReferenceRule::Zero)
        }
        (
            ControlFamily::DirectAttitudeThrust,
            ControlChannel::Roll | ControlChannel::Pitch | ControlChannel::Yaw,
        ) => (
            PhysicalUnit::Radians,
            ReferenceRule::EffectiveSetpointAtEntry,
        ),
        (ControlFamily::DirectAttitudeThrust, ControlChannel::Vertical) => (
            PhysicalUnit::NormalizedCollectiveForce,
            ReferenceRule::IdentifiedHoverTrim,
        ),
    }
}

const fn channel_name(channel: ControlChannel) -> &'static str {
    match channel {
        ControlChannel::Roll => "roll",
        ControlChannel::Pitch => "pitch",
        ControlChannel::Vertical => "vertical",
        ControlChannel::Yaw => "yaw",
    }
}
