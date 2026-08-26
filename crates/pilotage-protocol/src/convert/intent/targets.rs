//! The two target vocabularies a typed action can carry.
//!
//! A flight mode changes what the vehicle does with a demand; a feel mode
//! changes how the demand is shaped on its way there. They are converted
//! separately because they are separate meanings, and a request carrying the
//! wrong one fails closed rather than being read past.

use crate::wire;
use crate::{FeelTarget, ModeTarget};

use super::super::ConvertError;

pub(crate) fn feel_target_to_wire(target: FeelTarget) -> wire::FeelTarget {
    match target {
        FeelTarget::Precision => wire::FeelTarget::Precision,
        FeelTarget::Balanced => wire::FeelTarget::Balanced,
        FeelTarget::Agile => wire::FeelTarget::Agile,
    }
}

pub(crate) fn feel_target_from_wire(value: i32) -> Result<FeelTarget, ConvertError> {
    match wire::FeelTarget::try_from(value) {
        Ok(wire::FeelTarget::Precision) => Ok(FeelTarget::Precision),
        Ok(wire::FeelTarget::Balanced) => Ok(FeelTarget::Balanced),
        Ok(wire::FeelTarget::Agile) => Ok(FeelTarget::Agile),
        // A feel request with no target is a request the receiver would have
        // to guess at, and guessing which law to install is exactly what the
        // typed vocabulary exists to prevent.
        Ok(wire::FeelTarget::Unspecified) | Err(_) => Err(ConvertError::UnknownEnum {
            enum_name: "pilotage.v1.FeelTarget",
            value,
        }),
    }
}

pub(crate) fn mode_target_to_wire(target: ModeTarget) -> wire::ModeTarget {
    match target {
        ModeTarget::CameraVelocity => wire::ModeTarget::CameraVelocity,
        ModeTarget::FpvDirect => wire::ModeTarget::FpvDirect,
        ModeTarget::Hold => wire::ModeTarget::Hold,
        ModeTarget::Return => wire::ModeTarget::Return,
    }
}

pub(crate) fn mode_target_from_wire(value: i32) -> Result<ModeTarget, ConvertError> {
    match wire::ModeTarget::try_from(value) {
        Ok(wire::ModeTarget::CameraVelocity) => Ok(ModeTarget::CameraVelocity),
        Ok(wire::ModeTarget::FpvDirect) => Ok(ModeTarget::FpvDirect),
        Ok(wire::ModeTarget::Hold) => Ok(ModeTarget::Hold),
        Ok(wire::ModeTarget::Return) => Ok(ModeTarget::Return),
        Ok(wire::ModeTarget::Unspecified) | Err(_) => Err(ConvertError::UnknownEnum {
            enum_name: "pilotage.v1.ModeTarget",
            value,
        }),
    }
}
