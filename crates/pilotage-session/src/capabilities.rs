//! Projection of adapter capabilities into the `HostCapabilities` wire type
//! and registration of their scopes with the authority engine (ADR-0006,
//! ADR-0008).
//!
//! The adapter (`pilotage-adapter-api`) and the wire (`pilotage-protocol`)
//! model capabilities differently: the adapter lists logical axes per scope
//! and link-loss actions per vehicle, while the wire carries display names and
//! a single link-loss action per scope. This module is the one place that
//! bridge lives, so the engine and the driver never hand-roll the mapping.

use pilotage_adapter_api::{
    AdapterCapabilities, LinkLossPolicy, ScopeDescriptor as AdapterScope,
    VehicleDescriptor as AdapterVehicle,
};
use pilotage_protocol::wire;

/// Builds the wire `HostCapabilities` a `ServerWelcome` advertises from the
/// adapter's capability report and the configured host version.
#[must_use]
pub(crate) fn host_capabilities(
    caps: &AdapterCapabilities,
    host_version: &str,
) -> wire::HostCapabilities {
    wire::HostCapabilities {
        host_version: host_version.to_owned(),
        vehicles: caps.vehicles.iter().map(vehicle_descriptor).collect(),
        supported_modes: execution_modes(caps),
    }
}

/// The adapter-declared descriptor for one `(vehicle, scope)` pair, when the
/// capabilities report it.
pub(crate) fn scope_capability<'a>(
    caps: &'a AdapterCapabilities,
    vehicle: pilotage_protocol::VehicleId,
    scope: &pilotage_protocol::ScopeId,
) -> Option<&'a AdapterScope> {
    caps.vehicles
        .iter()
        .find(|descriptor| descriptor.id == vehicle)?
        .scopes
        .iter()
        .find(|descriptor| descriptor.scope == *scope)
}

/// Enumerates the `(vehicle, scope)` pairs the adapter exposes so the engine
/// can register each with the authority engine at construction.
pub(crate) fn scope_pairs(
    caps: &AdapterCapabilities,
) -> impl Iterator<Item = (pilotage_protocol::VehicleId, pilotage_protocol::ScopeId)> + '_ {
    caps.vehicles.iter().flat_map(|vehicle| {
        vehicle
            .scopes
            .iter()
            .map(move |scope| (vehicle.id, scope.scope.clone()))
    })
}

fn vehicle_descriptor(vehicle: &AdapterVehicle) -> wire::VehicleDescriptor {
    let link_loss = vehicle
        .link_loss_actions
        .first()
        .copied()
        .map_or(wire::LinkLossAction::Unspecified, link_loss_action);
    wire::VehicleDescriptor {
        vehicle: Some(wire::VehicleId {
            value: vehicle.id.as_u64(),
        }),
        display_name: String::new(),
        scopes: vehicle
            .scopes
            .iter()
            .map(|scope| scope_descriptor(scope, link_loss))
            .collect(),
        supported_modes: Vec::new(),
    }
}

fn scope_descriptor(
    scope: &AdapterScope,
    link_loss: wire::LinkLossAction,
) -> wire::ScopeDescriptor {
    wire::ScopeDescriptor {
        scope: Some(wire::ScopeId {
            value: scope.scope.as_str().to_owned(),
        }),
        display_name: String::new(),
        link_loss_action: link_loss as i32,
        intents: scope.intents.iter().map(intent_capability).collect(),
        actions: scope.actions.iter().map(action_capability).collect(),
    }
}

/// Projects one adapter-declared intent capability onto the wire, so a
/// client scales its typed commands by the REAL envelope the adapter
/// enforces (CTRL-01).
fn intent_capability(intent: &pilotage_adapter_api::IntentCapability) -> wire::IntentCapability {
    wire::IntentCapability {
        family: intent_family(intent.family) as i32,
        frames: intent
            .frames
            .iter()
            .map(|frame| reference_frame(*frame) as i32)
            .collect(),
        max_linear: intent.max_linear,
        max_angular: intent.max_angular,
        max_vertical: intent.max_vertical,
    }
}

fn action_capability(action: &pilotage_adapter_api::ActionCapability) -> wire::ActionCapability {
    wire::ActionCapability {
        action: action_kind(action.action) as i32,
        mode_targets: action
            .mode_targets
            .iter()
            .map(|target| mode_target(*target) as i32)
            .collect(),
    }
}

fn intent_family(family: pilotage_protocol::IntentFamily) -> wire::IntentFamily {
    use pilotage_protocol::IntentFamily as Domain;
    match family {
        Domain::Velocity => wire::IntentFamily::Velocity,
        Domain::PositionHold => wire::IntentFamily::PositionHold,
        Domain::AttitudeThrust => wire::IntentFamily::AttitudeThrust,
        Domain::BodyRate => wire::IntentFamily::BodyRate,
        Domain::GimbalRate => wire::IntentFamily::GimbalRate,
    }
}

fn reference_frame(frame: pilotage_protocol::ReferenceFrame) -> wire::ReferenceFrame {
    use pilotage_protocol::ReferenceFrame as Domain;
    match frame {
        Domain::BodyFrd => wire::ReferenceFrame::BodyFrd,
        Domain::LocalNed => wire::ReferenceFrame::LocalNed,
        Domain::Gimbal => wire::ReferenceFrame::Gimbal,
    }
}

fn action_kind(kind: pilotage_protocol::ActionKind) -> wire::ControlAction {
    use pilotage_protocol::ActionKind as Domain;
    match kind {
        Domain::Arm => wire::ControlAction::Arm,
        Domain::Disarm => wire::ControlAction::Disarm,
        Domain::ModeRequest => wire::ControlAction::ModeRequest,
        Domain::GimbalRecenter => wire::ControlAction::GimbalRecenter,
        Domain::SimReset => wire::ControlAction::SimReset,
    }
}

fn mode_target(target: pilotage_protocol::ModeTarget) -> wire::ModeTarget {
    use pilotage_protocol::ModeTarget as Domain;
    match target {
        Domain::CameraVelocity => wire::ModeTarget::CameraVelocity,
        Domain::FpvDirect => wire::ModeTarget::FpvDirect,
        Domain::Hold => wire::ModeTarget::Hold,
        Domain::Return => wire::ModeTarget::Return,
    }
}

/// Projects the adapter's execution-mode flags onto the repeated wire enum.
///
/// The wire enum omits `deterministic`, `render_capable`, and
/// `physically_embodied`; those are engine-internal characteristics the client
/// does not select a session mode from, so only the run-time cadence flags
/// (`real_time`, `accelerated`, `stepped`) are advertised.
fn execution_modes(caps: &AdapterCapabilities) -> Vec<i32> {
    let mut modes = Vec::new();
    if caps.execution.real_time {
        modes.push(wire::ExecutionMode::Realtime as i32);
    }
    if caps.execution.accelerated {
        modes.push(wire::ExecutionMode::Accelerated as i32);
    }
    if caps.execution.stepped {
        modes.push(wire::ExecutionMode::Stepped as i32);
    }
    modes
}

fn link_loss_action(policy: LinkLossPolicy) -> wire::LinkLossAction {
    match policy {
        LinkLossPolicy::Neutralize => wire::LinkLossAction::Neutral,
        LinkLossPolicy::Brake => wire::LinkLossAction::Stop,
        LinkLossPolicy::HoldBrief { .. } => wire::LinkLossAction::HoldLast,
        LinkLossPolicy::Pause => wire::LinkLossAction::Stop,
        LinkLossPolicy::EngageAutomation => wire::LinkLossAction::RevokeScope,
    }
}
