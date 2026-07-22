//! Wire ↔ domain conversions for frame rejections and their typed reasons,
//! mirroring `session_convert`'s pattern.

use crate::convert::ConvertError;
use crate::ids::{Generation, ScopeId, SequenceNum, VehicleId};
use crate::session::{FrameRejected, FrameRejectionReason};
use crate::wire;

impl From<FrameRejectionReason> for wire::FrameRejectionReason {
    fn from(reason: FrameRejectionReason) -> Self {
        match reason {
            FrameRejectionReason::StaleGeneration => wire::FrameRejectionReason::StaleGeneration,
            FrameRejectionReason::NoHolder => wire::FrameRejectionReason::NoHolder,
            FrameRejectionReason::UnknownScope => wire::FrameRejectionReason::UnknownScope,
            FrameRejectionReason::TooOld => wire::FrameRejectionReason::TooOld,
            FrameRejectionReason::EmptyCommand => wire::FrameRejectionReason::EmptyCommand,
            FrameRejectionReason::DualCommand => wire::FrameRejectionReason::DualCommand,
            FrameRejectionReason::UnsupportedIntent => {
                wire::FrameRejectionReason::UnsupportedIntent
            }
            FrameRejectionReason::UnsupportedAction => {
                wire::FrameRejectionReason::UnsupportedAction
            }
            FrameRejectionReason::LimitExceeded => wire::FrameRejectionReason::LimitExceeded,
            FrameRejectionReason::ConflictingActions => {
                wire::FrameRejectionReason::ConflictingActions
            }
            FrameRejectionReason::PartialCommand => wire::FrameRejectionReason::PartialCommand,
            FrameRejectionReason::ProfileMismatch => wire::FrameRejectionReason::ProfileMismatch,
            FrameRejectionReason::ActionOnDatagram => wire::FrameRejectionReason::ActionOnDatagram,
        }
    }
}

impl TryFrom<wire::FrameRejectionReason> for FrameRejectionReason {
    type Error = ConvertError;

    fn try_from(reason: wire::FrameRejectionReason) -> Result<Self, Self::Error> {
        match reason {
            wire::FrameRejectionReason::StaleGeneration => {
                Ok(FrameRejectionReason::StaleGeneration)
            }
            wire::FrameRejectionReason::NoHolder => Ok(FrameRejectionReason::NoHolder),
            wire::FrameRejectionReason::UnknownScope => Ok(FrameRejectionReason::UnknownScope),
            wire::FrameRejectionReason::TooOld => Ok(FrameRejectionReason::TooOld),
            wire::FrameRejectionReason::EmptyCommand => Ok(FrameRejectionReason::EmptyCommand),
            wire::FrameRejectionReason::DualCommand => Ok(FrameRejectionReason::DualCommand),
            wire::FrameRejectionReason::UnsupportedIntent => {
                Ok(FrameRejectionReason::UnsupportedIntent)
            }
            wire::FrameRejectionReason::UnsupportedAction => {
                Ok(FrameRejectionReason::UnsupportedAction)
            }
            wire::FrameRejectionReason::LimitExceeded => Ok(FrameRejectionReason::LimitExceeded),
            wire::FrameRejectionReason::ConflictingActions => {
                Ok(FrameRejectionReason::ConflictingActions)
            }
            wire::FrameRejectionReason::PartialCommand => Ok(FrameRejectionReason::PartialCommand),
            wire::FrameRejectionReason::ProfileMismatch => {
                Ok(FrameRejectionReason::ProfileMismatch)
            }
            wire::FrameRejectionReason::ActionOnDatagram => {
                Ok(FrameRejectionReason::ActionOnDatagram)
            }
            wire::FrameRejectionReason::Unspecified => Err(ConvertError::UnknownEnum {
                enum_name: "pilotage.v1.FrameRejectionReason",
                value: reason as i32,
            }),
        }
    }
}

impl From<&FrameRejected> for wire::FrameRejected {
    fn from(rejected: &FrameRejected) -> Self {
        wire::FrameRejected {
            vehicle: Some(wire::VehicleId {
                value: rejected.vehicle.as_u64(),
            }),
            scope: Some(wire::ScopeId {
                value: rejected.scope.as_str().to_owned(),
            }),
            sequence: Some(wire::SequenceNum {
                value: rejected.sequence.as_u32(),
            }),
            reason: wire::FrameRejectionReason::from(rejected.reason) as i32,
            current_generation: Some(wire::Generation {
                value: rejected.current_generation.as_u64(),
            }),
        }
    }
}

impl TryFrom<wire::FrameRejected> for FrameRejected {
    type Error = ConvertError;

    fn try_from(rejected: wire::FrameRejected) -> Result<Self, Self::Error> {
        let missing = |field: &'static str| ConvertError::MissingField {
            message: "pilotage.v1.FrameRejected",
            field,
        };
        let vehicle = rejected.vehicle.ok_or_else(|| missing("vehicle"))?;
        let scope = rejected.scope.ok_or_else(|| missing("scope"))?;
        let sequence = rejected.sequence.ok_or_else(|| missing("sequence"))?;
        let current_generation = rejected
            .current_generation
            .ok_or_else(|| missing("current_generation"))?;
        let raw_reason = wire::FrameRejectionReason::try_from(rejected.reason).map_err(|_| {
            ConvertError::UnknownEnum {
                enum_name: "pilotage.v1.FrameRejectionReason",
                value: rejected.reason,
            }
        })?;
        Ok(FrameRejected {
            vehicle: VehicleId::new(vehicle.value),
            scope: ScopeId::new(scope.value),
            sequence: SequenceNum::new(sequence.value),
            reason: FrameRejectionReason::try_from(raw_reason)?,
            current_generation: Generation::new(current_generation.value),
        })
    }
}
