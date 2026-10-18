//! The capability advertisement: the flight envelope the uplink enforces,
//! the typed actions the flight scope consumes, and the legacy translation
//! map the session host's compatibility boundary uses (CTRL-01). No
//! simulator lifecycle scope: this adapter has no reset path.

use pilotage_adapter_api::{
    ActionCapability, AdapterCapabilities, ExecutionMode, IntentCapability, LegacyAxisRoute,
    LegacyCommandMap, LinkLossPolicy, ScopeDescriptor, VehicleDescriptor,
};
use pilotage_protocol::{ActionKind, IntentFamily, LogicalAxisId, ReferenceFrame, ScopeId};

use super::{
    ARM_BUTTON, ArduPilotAdapter, DISARM_BUTTON, FLIGHT_SCOPE, PITCH_AXIS, ROLL_AXIS,
    THROTTLE_AXIS, YAW_AXIS,
};
use crate::uplink::{MAX_HORIZONTAL_MPS, MAX_VERTICAL_MPS, MAX_YAW_RATE_RPS};

impl ArduPilotAdapter {
    /// The capability report `VehicleAdapter::capabilities` returns.
    pub(super) fn advertised_capabilities(&self) -> AdapterCapabilities {
        let flying = self.uplink.is_some();
        AdapterCapabilities {
            execution: ExecutionMode {
                real_time: true,
                ..ExecutionMode::default()
            },
            vehicles: vec![VehicleDescriptor {
                id: self.vehicle,
                scopes: if flying {
                    vec![flight_scope_descriptor()]
                } else {
                    vec![]
                },
                link_loss_actions: if flying {
                    vec![LinkLossPolicy::Neutralize]
                } else {
                    vec![]
                },
            }],
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
            control_feel: None,
        }
    }
}

fn action(action: ActionKind) -> ActionCapability {
    ActionCapability {
        action,
        mode_targets: vec![],
        feel_targets: vec![],
    }
}

fn flight_scope_descriptor() -> ScopeDescriptor {
    ScopeDescriptor {
        authority_group: None,
        scope: ScopeId::new(FLIGHT_SCOPE),
        axes: [ROLL_AXIS, PITCH_AXIS, THROTTLE_AXIS, YAW_AXIS]
            .map(LogicalAxisId::new)
            .to_vec(),
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
