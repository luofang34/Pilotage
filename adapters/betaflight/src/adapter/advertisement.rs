//! The capability advertisement: one flight scope taking BODY_FRD velocity
//! intents inside the envelope the outer loop flies, plus ARM and DISARM.

use pilotage_adapter_api::{
    ActionCapability, AdapterCapabilities, ExecutionMode, IntentCapability, LegacyAxisRoute,
    LegacyCommandMap, LinkLossPolicy, ScopeDescriptor, VehicleDescriptor,
};
use pilotage_protocol::{ActionKind, IntentFamily, LogicalAxisId, ReferenceFrame, ScopeId};

use super::{
    ARM_BUTTON, BetaflightAdapter, DISARM_BUTTON, FLIGHT_SCOPE, PITCH_AXIS, ROLL_AXIS,
    THROTTLE_AXIS, YAW_AXIS,
};
use crate::config::{MAX_HORIZONTAL_MPS, MAX_VERTICAL_MPS, MAX_YAW_RATE_RPS};

impl BetaflightAdapter {
    /// The capability report `VehicleAdapter::capabilities` returns.
    pub(super) fn advertised_capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            execution: ExecutionMode {
                real_time: true,
                ..ExecutionMode::default()
            },
            vehicles: vec![VehicleDescriptor {
                id: self.vehicle,
                scopes: vec![flight_scope_descriptor()],
                link_loss_actions: vec![LinkLossPolicy::Neutralize],
            }],
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
            control_feel: None,
        }
    }
}

fn flight_scope_descriptor() -> ScopeDescriptor {
    let action = |action| ActionCapability {
        action,
        mode_targets: vec![],
        feel_targets: vec![],
    };
    ScopeDescriptor {
        authority_group: None,
        scope: ScopeId::new(FLIGHT_SCOPE),
        axes: vec![
            LogicalAxisId::new(ROLL_AXIS),
            LogicalAxisId::new(PITCH_AXIS),
            LogicalAxisId::new(THROTTLE_AXIS),
            LogicalAxisId::new(YAW_AXIS),
        ],
        // The envelope the outer loop flies; a full-scale demand is these.
        intents: vec![IntentCapability {
            max_yaw_rate: 0.0,
            family: IntentFamily::Velocity,
            frames: vec![ReferenceFrame::BodyFrd],
            max_linear: MAX_HORIZONTAL_MPS,
            max_vertical: MAX_VERTICAL_MPS,
            max_angular: MAX_YAW_RATE_RPS,
        }],
        actions: vec![action(ActionKind::Arm), action(ActionKind::Disarm)],
        legacy: Some(LegacyCommandMap::Velocity {
            vx: Some(LegacyAxisRoute {
                axis: PITCH_AXIS,
                sign: 1.0,
            }),
            vy: Some(LegacyAxisRoute {
                axis: ROLL_AXIS,
                sign: 1.0,
            }),
            // + throttle = climb; body-FRD +z is down.
            vz: Some(LegacyAxisRoute {
                axis: THROTTLE_AXIS,
                sign: -1.0,
            }),
            yaw_rate: Some(LegacyAxisRoute {
                axis: YAW_AXIS,
                sign: 1.0,
            }),
            arm_button: Some(ARM_BUTTON),
            disarm_button: Some(DISARM_BUTTON),
        }),
    }
}
