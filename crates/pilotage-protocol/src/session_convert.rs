//! Conversions between session-bootstrap domain types (`session.rs`) and
//! generated wire types (`wire.rs`), mirroring `convert.rs`'s pattern
//! (ADR-0005, ADR-0014).

use crate::convert::ConvertError;
use crate::ids::{Generation, PrincipalId, ScopeId, SequenceNum, SessionId, VehicleId};
use crate::session::{
    ClientHello, FrameRejected, FrameRejectionReason, LeaseDenialReason, LeaseRelease,
    LeaseReleased, LeaseRequest, LeaseResponse, LinkLossCleared, Ping, Pong, ScopeHolderSnapshot,
    ServerWelcome,
};
use crate::wire;
use pilotage_timing::MonoTimestamp;

impl From<&ClientHello> for wire::ClientHello {
    fn from(hello: &ClientHello) -> Self {
        wire::ClientHello {
            protocol_version: hello.protocol_version,
            client_name: hello.client_name.clone(),
            join_token: hello.join_token.clone(),
        }
    }
}

impl From<wire::ClientHello> for ClientHello {
    fn from(hello: wire::ClientHello) -> Self {
        ClientHello {
            protocol_version: hello.protocol_version,
            client_name: hello.client_name,
            join_token: hello.join_token,
        }
    }
}

impl From<&ScopeHolderSnapshot> for wire::ScopeHolderSnapshot {
    fn from(snapshot: &ScopeHolderSnapshot) -> Self {
        wire::ScopeHolderSnapshot {
            vehicle: Some(wire::VehicleId {
                value: snapshot.vehicle.as_u64(),
            }),
            scope: Some(wire::ScopeId {
                value: snapshot.scope.as_str().to_owned(),
            }),
            holder: snapshot.holder.map(|principal| wire::PrincipalId {
                value: principal.as_u64(),
            }),
            generation: Some(wire::Generation {
                value: snapshot.generation.as_u64(),
            }),
        }
    }
}

impl TryFrom<wire::ScopeHolderSnapshot> for ScopeHolderSnapshot {
    type Error = ConvertError;

    fn try_from(snapshot: wire::ScopeHolderSnapshot) -> Result<Self, Self::Error> {
        let missing = |field: &'static str| ConvertError::MissingField {
            message: "pilotage.v1.ScopeHolderSnapshot",
            field,
        };
        let vehicle = snapshot.vehicle.ok_or_else(|| missing("vehicle"))?;
        let scope = snapshot.scope.ok_or_else(|| missing("scope"))?;
        let generation = snapshot.generation.ok_or_else(|| missing("generation"))?;
        Ok(ScopeHolderSnapshot {
            vehicle: VehicleId::new(vehicle.value),
            scope: ScopeId::new(scope.value),
            holder: snapshot.holder.map(|holder| PrincipalId::new(holder.value)),
            generation: Generation::new(generation.value),
        })
    }
}

impl From<&ServerWelcome> for wire::ServerWelcome {
    fn from(welcome: &ServerWelcome) -> Self {
        wire::ServerWelcome {
            session: Some(wire::SessionId {
                value: welcome.session.as_u64(),
            }),
            principal: Some(wire::PrincipalId {
                value: welcome.principal.as_u64(),
            }),
            host_capabilities: Some(welcome.host_capabilities.clone()),
            scope_holders: welcome.scope_holders.iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<wire::ServerWelcome> for ServerWelcome {
    type Error = ConvertError;

    fn try_from(welcome: wire::ServerWelcome) -> Result<Self, Self::Error> {
        let missing = |field: &'static str| ConvertError::MissingField {
            message: "pilotage.v1.ServerWelcome",
            field,
        };
        let session = welcome.session.ok_or_else(|| missing("session"))?;
        let principal = welcome.principal.ok_or_else(|| missing("principal"))?;
        let host_capabilities = welcome
            .host_capabilities
            .ok_or_else(|| missing("host_capabilities"))?;
        let scope_holders = welcome
            .scope_holders
            .into_iter()
            .map(ScopeHolderSnapshot::try_from)
            .collect::<Result<_, ConvertError>>()?;
        Ok(ServerWelcome {
            session: SessionId::new(session.value),
            principal: PrincipalId::new(principal.value),
            host_capabilities,
            scope_holders,
        })
    }
}

impl From<&LeaseRequest> for wire::LeaseRequest {
    fn from(request: &LeaseRequest) -> Self {
        wire::LeaseRequest {
            vehicle: Some(wire::VehicleId {
                value: request.vehicle.as_u64(),
            }),
            scope: Some(wire::ScopeId {
                value: request.scope.as_str().to_owned(),
            }),
        }
    }
}

impl TryFrom<wire::LeaseRequest> for LeaseRequest {
    type Error = ConvertError;

    fn try_from(request: wire::LeaseRequest) -> Result<Self, Self::Error> {
        let missing = |field: &'static str| ConvertError::MissingField {
            message: "pilotage.v1.LeaseRequest",
            field,
        };
        let vehicle = request.vehicle.ok_or_else(|| missing("vehicle"))?;
        let scope = request.scope.ok_or_else(|| missing("scope"))?;
        Ok(LeaseRequest {
            vehicle: VehicleId::new(vehicle.value),
            scope: ScopeId::new(scope.value),
        })
    }
}

impl TryFrom<wire::LeaseRelease> for LeaseRelease {
    type Error = ConvertError;

    fn try_from(release: wire::LeaseRelease) -> Result<Self, Self::Error> {
        let missing = |field: &'static str| ConvertError::MissingField {
            message: "pilotage.v1.LeaseRelease",
            field,
        };
        let vehicle = release.vehicle.ok_or_else(|| missing("vehicle"))?;
        let scope = release.scope.ok_or_else(|| missing("scope"))?;
        Ok(LeaseRelease {
            vehicle: VehicleId::new(vehicle.value),
            scope: ScopeId::new(scope.value),
        })
    }
}

impl From<&LeaseRelease> for wire::LeaseRelease {
    fn from(release: &LeaseRelease) -> Self {
        wire::LeaseRelease {
            vehicle: Some(wire::VehicleId {
                value: release.vehicle.as_u64(),
            }),
            scope: Some(wire::ScopeId {
                value: release.scope.as_str().to_owned(),
            }),
        }
    }
}

impl From<&LeaseReleased> for wire::LeaseReleased {
    fn from(released: &LeaseReleased) -> Self {
        wire::LeaseReleased {
            vehicle: Some(wire::VehicleId {
                value: released.vehicle.as_u64(),
            }),
            scope: Some(wire::ScopeId {
                value: released.scope.as_str().to_owned(),
            }),
            released: released.released,
            generation: Some(wire::Generation {
                value: released.generation.as_u64(),
            }),
        }
    }
}

impl TryFrom<wire::LeaseReleased> for LeaseReleased {
    type Error = ConvertError;

    fn try_from(released: wire::LeaseReleased) -> Result<Self, Self::Error> {
        let missing = |field: &'static str| ConvertError::MissingField {
            message: "pilotage.v1.LeaseReleased",
            field,
        };
        let vehicle = released.vehicle.ok_or_else(|| missing("vehicle"))?;
        let scope = released.scope.ok_or_else(|| missing("scope"))?;
        let generation = released.generation.ok_or_else(|| missing("generation"))?;
        Ok(LeaseReleased {
            vehicle: VehicleId::new(vehicle.value),
            scope: ScopeId::new(scope.value),
            released: released.released,
            generation: Generation::new(generation.value),
        })
    }
}

impl From<&LinkLossCleared> for wire::LinkLossCleared {
    fn from(cleared: &LinkLossCleared) -> Self {
        wire::LinkLossCleared {
            vehicle: Some(wire::VehicleId {
                value: cleared.vehicle.as_u64(),
            }),
            scope: Some(wire::ScopeId {
                value: cleared.scope.as_str().to_owned(),
            }),
            generation: Some(wire::Generation {
                value: cleared.generation.as_u64(),
            }),
        }
    }
}

impl TryFrom<wire::LinkLossCleared> for LinkLossCleared {
    type Error = ConvertError;

    fn try_from(cleared: wire::LinkLossCleared) -> Result<Self, Self::Error> {
        let missing = |field: &'static str| ConvertError::MissingField {
            message: "pilotage.v1.LinkLossCleared",
            field,
        };
        let vehicle = cleared.vehicle.ok_or_else(|| missing("vehicle"))?;
        let scope = cleared.scope.ok_or_else(|| missing("scope"))?;
        let generation = cleared.generation.ok_or_else(|| missing("generation"))?;
        Ok(LinkLossCleared {
            vehicle: VehicleId::new(vehicle.value),
            scope: ScopeId::new(scope.value),
            generation: Generation::new(generation.value),
        })
    }
}

impl From<LeaseDenialReason> for wire::LeaseDenialReason {
    fn from(reason: LeaseDenialReason) -> Self {
        match reason {
            LeaseDenialReason::AlreadyHeld => wire::LeaseDenialReason::AlreadyHeld,
            LeaseDenialReason::UnknownScope => wire::LeaseDenialReason::UnknownScope,
            LeaseDenialReason::NotAuthorized => wire::LeaseDenialReason::NotAuthorized,
        }
    }
}

impl TryFrom<wire::LeaseDenialReason> for LeaseDenialReason {
    type Error = ConvertError;

    fn try_from(reason: wire::LeaseDenialReason) -> Result<Self, Self::Error> {
        match reason {
            wire::LeaseDenialReason::AlreadyHeld => Ok(LeaseDenialReason::AlreadyHeld),
            wire::LeaseDenialReason::UnknownScope => Ok(LeaseDenialReason::UnknownScope),
            wire::LeaseDenialReason::NotAuthorized => Ok(LeaseDenialReason::NotAuthorized),
            wire::LeaseDenialReason::Unspecified => Err(ConvertError::UnknownEnum {
                enum_name: "pilotage.v1.LeaseDenialReason",
                value: reason as i32,
            }),
        }
    }
}

impl From<&LeaseResponse> for wire::LeaseResponse {
    fn from(response: &LeaseResponse) -> Self {
        wire::LeaseResponse {
            vehicle: Some(wire::VehicleId {
                value: response.vehicle.as_u64(),
            }),
            scope: Some(wire::ScopeId {
                value: response.scope.as_str().to_owned(),
            }),
            granted: response.granted,
            generation: Some(wire::Generation {
                value: response.generation.as_u64(),
            }),
            reason: response.reason.map_or(
                wire::LeaseDenialReason::Unspecified,
                wire::LeaseDenialReason::from,
            ) as i32,
        }
    }
}

impl TryFrom<wire::LeaseResponse> for LeaseResponse {
    type Error = ConvertError;

    fn try_from(response: wire::LeaseResponse) -> Result<Self, Self::Error> {
        let missing = |field: &'static str| ConvertError::MissingField {
            message: "pilotage.v1.LeaseResponse",
            field,
        };
        let vehicle = response.vehicle.ok_or_else(|| missing("vehicle"))?;
        let scope = response.scope.ok_or_else(|| missing("scope"))?;
        let generation = response.generation.ok_or_else(|| missing("generation"))?;
        let raw_reason = wire::LeaseDenialReason::try_from(response.reason).map_err(|_| {
            ConvertError::UnknownEnum {
                enum_name: "pilotage.v1.LeaseDenialReason",
                value: response.reason,
            }
        })?;
        // Denial reason is only meaningful when the lease was refused; a
        // grant response is not required to carry a concrete reason.
        let reason = if response.granted {
            None
        } else {
            Some(LeaseDenialReason::try_from(raw_reason)?)
        };
        Ok(LeaseResponse {
            vehicle: VehicleId::new(vehicle.value),
            scope: ScopeId::new(scope.value),
            granted: response.granted,
            generation: Generation::new(generation.value),
            reason,
        })
    }
}

impl From<&Ping> for wire::Ping {
    fn from(ping: &Ping) -> Self {
        wire::Ping {
            nonce: ping.nonce,
            sender_sent_at: Some(wire::MonoTimestamp {
                nanos: ping.sender_sent_at.as_nanos(),
            }),
        }
    }
}

impl TryFrom<wire::Ping> for Ping {
    type Error = ConvertError;

    fn try_from(ping: wire::Ping) -> Result<Self, Self::Error> {
        let sender_sent_at = ping.sender_sent_at.ok_or(ConvertError::MissingField {
            message: "pilotage.v1.Ping",
            field: "sender_sent_at",
        })?;
        Ok(Ping {
            nonce: ping.nonce,
            sender_sent_at: MonoTimestamp::from_nanos(sender_sent_at.nanos),
        })
    }
}

impl From<&Pong> for wire::Pong {
    fn from(pong: &Pong) -> Self {
        wire::Pong {
            nonce: pong.nonce,
            echoed_sender_sent_at: Some(wire::MonoTimestamp {
                nanos: pong.echoed_sender_sent_at.as_nanos(),
            }),
            responder_sent_at: Some(wire::MonoTimestamp {
                nanos: pong.responder_sent_at.as_nanos(),
            }),
        }
    }
}

impl TryFrom<wire::Pong> for Pong {
    type Error = ConvertError;

    fn try_from(pong: wire::Pong) -> Result<Self, Self::Error> {
        let missing = |field: &'static str| ConvertError::MissingField {
            message: "pilotage.v1.Pong",
            field,
        };
        let echoed_sender_sent_at = pong
            .echoed_sender_sent_at
            .ok_or_else(|| missing("echoed_sender_sent_at"))?;
        let responder_sent_at = pong
            .responder_sent_at
            .ok_or_else(|| missing("responder_sent_at"))?;
        Ok(Pong {
            nonce: pong.nonce,
            echoed_sender_sent_at: MonoTimestamp::from_nanos(echoed_sender_sent_at.nanos),
            responder_sent_at: MonoTimestamp::from_nanos(responder_sent_at.nanos),
        })
    }
}

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

mod control_results;

#[cfg(test)]
mod tests;
