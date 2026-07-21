//! Wire ↔ domain conversions for the typed control-result vocabulary:
//! per-action outcomes and profile-activation announcements (CTRL-01,
//! INPUT-01), mirroring `session_convert`'s pattern.

use crate::convert::ConvertError;
use crate::ids::{Generation, ScopeId, SequenceNum, SessionId, VehicleId};
use crate::intent::ControlAction;
use crate::session::{ControlActionResult, ProfileActivation};
use crate::wire;

impl From<&ControlActionResult> for wire::ControlActionResult {
    fn from(result: &ControlActionResult) -> Self {
        let request = crate::convert::action_request_to_wire(result.action);
        wire::ControlActionResult {
            vehicle: Some(wire::VehicleId {
                value: result.vehicle.as_u64(),
            }),
            scope: Some(wire::ScopeId {
                value: result.scope.as_str().to_owned(),
            }),
            generation: Some(wire::Generation {
                value: result.generation.as_u64(),
            }),
            sequence: Some(wire::SequenceNum {
                value: result.sequence.as_u32(),
            }),
            action: request.action,
            mode_target: request.mode_target,
            accepted: result.accepted,
            detail: result.detail.clone(),
        }
    }
}

impl TryFrom<wire::ControlActionResult> for ControlActionResult {
    type Error = ConvertError;

    fn try_from(result: wire::ControlActionResult) -> Result<Self, Self::Error> {
        let missing = |field: &'static str| ConvertError::MissingField {
            message: "pilotage.v1.ControlActionResult",
            field,
        };
        let vehicle = result.vehicle.ok_or_else(|| missing("vehicle"))?;
        let scope = result.scope.ok_or_else(|| missing("scope"))?;
        let generation = result.generation.ok_or_else(|| missing("generation"))?;
        let sequence = result.sequence.ok_or_else(|| missing("sequence"))?;
        let action: ControlAction =
            crate::convert::action_request_from_wire(wire::ControlActionRequest {
                action: result.action,
                mode_target: result.mode_target,
            })?;
        Ok(ControlActionResult {
            vehicle: VehicleId::new(vehicle.value),
            scope: ScopeId::new(scope.value),
            generation: Generation::new(generation.value),
            sequence: SequenceNum::new(sequence.value),
            action,
            accepted: result.accepted,
            detail: result.detail,
        })
    }
}

impl From<&ProfileActivation> for wire::ProfileActivation {
    fn from(activation: &ProfileActivation) -> Self {
        wire::ProfileActivation {
            session: Some(wire::SessionId {
                value: activation.session.as_u64(),
            }),
            profile_id: activation.profile_id.clone(),
            profile_revision: activation.profile_revision,
            activation_revision: activation.activation_revision,
            digest: activation.digest.to_vec(),
        }
    }
}

impl TryFrom<wire::ProfileActivation> for ProfileActivation {
    type Error = ConvertError;

    fn try_from(activation: wire::ProfileActivation) -> Result<Self, Self::Error> {
        let session = activation.session.ok_or(ConvertError::MissingField {
            message: "pilotage.v1.ProfileActivation",
            field: "session",
        })?;
        let digest: [u8; 32] =
            activation
                .digest
                .as_slice()
                .try_into()
                .map_err(|_| ConvertError::MissingField {
                    message: "pilotage.v1.ProfileActivation",
                    field: "digest (must be exactly 32 bytes)",
                })?;
        Ok(ProfileActivation {
            session: SessionId::new(session.value),
            profile_id: activation.profile_id,
            profile_revision: activation.profile_revision,
            activation_revision: activation.activation_revision,
            digest,
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::intent::ModeTarget;

    #[test]
    fn a_control_action_result_round_trips() {
        let result = ControlActionResult {
            vehicle: VehicleId::new(1),
            scope: ScopeId::new("vehicle.motion"),
            generation: Generation::new(4),
            sequence: SequenceNum::new(77),
            action: ControlAction::ModeRequest {
                target: ModeTarget::Hold,
            },
            accepted: false,
            detail: "mode not supported while disarmed".to_owned(),
        };
        let wire_result = wire::ControlActionResult::from(&result);
        assert_eq!(
            ControlActionResult::try_from(wire_result).expect("round-trips"),
            result
        );
    }

    #[test]
    fn a_profile_activation_round_trips_and_enforces_digest_length() {
        let activation = ProfileActivation {
            session: SessionId::new(9),
            profile_id: "builtin.gimbal.default".to_owned(),
            profile_revision: 3,
            activation_revision: 2,
            digest: [0xAB; 32],
        };
        let wire_activation = wire::ProfileActivation::from(&activation);
        assert_eq!(
            ProfileActivation::try_from(wire_activation).expect("round-trips"),
            activation
        );

        let mut short = wire::ProfileActivation::from(&activation);
        short.digest.truncate(31);
        assert!(ProfileActivation::try_from(short).is_err());
    }
}
