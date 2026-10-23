//! The advertised capabilities: the DJI Mini 3 scopes, intents, and
//! actions, under a simulated execution mode.

use pilotage_adapter_api::{
    ActionCapability, AdapterCapabilities, ExecutionMode, IntentCapability, LinkLossPolicy,
    ScopeDescriptor, VehicleDescriptor,
};
use pilotage_protocol::{
    ActionKind, HeadingReference, IntentFamily, ReferenceFrame, ScopeId, TakeoffStartState,
    VehicleClass, VehicleId,
};

use super::{GIMBAL_SCOPE, MOTION_SCOPE};
use crate::config::Limits;
use crate::gimbal::PITCH_RATE_LIMIT_RPS;

/// The simulated execution mode. `physically_embodied` is false: this is
/// the machine-readable difference from the real vehicle.
pub(super) fn execution(render_capable: bool) -> ExecutionMode {
    ExecutionMode {
        real_time: false,
        stepped: true,
        accelerated: true,
        deterministic: true,
        render_capable,
        physically_embodied: false,
    }
}

pub(super) fn capabilities(
    vehicle: VehicleId,
    display_name: &str,
    limits: &Limits,
    render_capable: bool,
) -> AdapterCapabilities {
    AdapterCapabilities {
        execution: execution(render_capable),
        vehicles: vec![VehicleDescriptor {
            id: vehicle,
            class: VehicleClass::Multicopter,
            display_name: display_name.to_owned(),
            scopes: vec![gimbal_scope(), flight_scope(limits)],
            link_loss_actions: vec![LinkLossPolicy::Neutralize],
            // The simulated heading is true north by construction.
            heading_reference: Some(HeadingReference::True),
        }],
        adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
        control_feel: None,
    }
}

fn action(action: ActionKind) -> ActionCapability {
    ActionCapability {
        action,
        mode_targets: Vec::new(),
        feel_targets: Vec::new(),
        takeoff_start_state: (action == ActionKind::Takeoff)
            .then_some(TakeoffStartState::MotorsStopped),
    }
}

fn gimbal_scope() -> ScopeDescriptor {
    ScopeDescriptor {
        // The pitch-rate bound is the intent's own limit; a payload
        // contract is not advertised.
        payload: None,
        authority_group: None,
        scope: ScopeId::new(GIMBAL_SCOPE),
        axes: Vec::new(),
        intents: vec![IntentCapability {
            family: IntentFamily::GimbalRate,
            frames: Vec::new(),
            max_linear: 0.0,
            max_vertical: 0.0,
            max_angular: PITCH_RATE_LIMIT_RPS,
            max_yaw_rate: 0.0,
        }],
        actions: vec![action(ActionKind::GimbalRecenter)],
        legacy: None,
    }
}

fn flight_scope(limits: &Limits) -> ScopeDescriptor {
    ScopeDescriptor {
        payload: None,
        authority_group: None,
        scope: ScopeId::new(MOTION_SCOPE),
        axes: Vec::new(),
        intents: vec![IntentCapability {
            family: IntentFamily::Velocity,
            frames: vec![ReferenceFrame::BodyFrd],
            max_linear: limits.horizontal_mps as f32,
            max_vertical: limits.vertical_mps as f32,
            max_angular: limits.yaw_rate_rps as f32,
            max_yaw_rate: 0.0,
        }],
        // Takeoff and Land are the DJI actions; Arm and Disarm are the
        // simulator mission path.
        actions: [
            ActionKind::Takeoff,
            ActionKind::Land,
            ActionKind::Arm,
            ActionKind::Disarm,
        ]
        .into_iter()
        .map(action)
        .collect(),
        legacy: None,
    }
}
