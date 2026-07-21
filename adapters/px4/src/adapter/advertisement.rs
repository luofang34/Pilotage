//! The PX4 adapter's capability advertisement: the REAL flight and gimbal
//! envelopes the uplink and gimbal command paths enforce, the typed actions
//! each scope consumes, and the legacy translation maps the session host's
//! compatibility boundary interprets numeric payloads through (CTRL-01).

use pilotage_adapter_api::{
    ActionCapability, AdapterCapabilities, ExecutionMode, IntentCapability, LegacyAxisRoute,
    LegacyCommandMap, LinkLossPolicy, ScopeDescriptor, VehicleDescriptor,
};
use pilotage_protocol::{ActionKind, IntentFamily, LogicalAxisId, ReferenceFrame, ScopeId};

use super::{
    ARM_BUTTON, DISARM_BUTTON, FLIGHT_SCOPE, GIMBAL_NEUTRAL_BUTTON, GIMBAL_SCOPE, PITCH_AXIS,
    Px4Adapter, RESET_BUTTON, ROLL_AXIS, THROTTLE_AXIS, YAW_AXIS,
};

impl Px4Adapter {
    /// The capability report `VehicleAdapter::capabilities` returns.
    pub(super) fn advertised_capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            execution: ExecutionMode {
                real_time: true,
                ..ExecutionMode::default()
            },
            vehicles: vec![VehicleDescriptor {
                id: self.vehicle,
                scopes: {
                    let mut scopes = Vec::new();
                    if self.uplink.is_some() {
                        scopes.push(flight_scope_descriptor());
                    }
                    if self.gimbal.is_some() {
                        scopes.push(gimbal_scope_descriptor());
                    }
                    scopes
                },
                link_loss_actions: if self.uplink.is_some() {
                    vec![LinkLossPolicy::Neutralize]
                } else {
                    vec![]
                },
            }],
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

fn flight_scope_descriptor() -> ScopeDescriptor {
    ScopeDescriptor {
        scope: ScopeId::new(FLIGHT_SCOPE),
        axes: vec![
            LogicalAxisId::new(ROLL_AXIS),
            LogicalAxisId::new(PITCH_AXIS),
            LogicalAxisId::new(THROTTLE_AXIS),
            LogicalAxisId::new(YAW_AXIS),
        ],
        // The REAL flight envelope the uplink enforces.
        intents: vec![IntentCapability {
            family: IntentFamily::Velocity,
            frames: vec![ReferenceFrame::BodyFrd],
            max_linear: crate::uplink::MAX_HORIZONTAL_MPS,
            max_vertical: crate::uplink::MAX_VERTICAL_MPS,
            max_angular: crate::uplink::MAX_YAW_RATE_RPS,
        }],
        actions: vec![
            ActionCapability {
                action: ActionKind::Arm,
                mode_targets: vec![],
            },
            ActionCapability {
                action: ActionKind::Disarm,
                mode_targets: vec![],
            },
            ActionCapability {
                action: ActionKind::SimReset,
                mode_targets: vec![],
            },
        ],
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
            reset_button: Some(RESET_BUTTON),
        }),
    }
}

fn gimbal_scope_descriptor() -> ScopeDescriptor {
    ScopeDescriptor {
        scope: ScopeId::new(GIMBAL_SCOPE),
        axes: vec![LogicalAxisId::new(PITCH_AXIS), LogicalAxisId::new(YAW_AXIS)],
        intents: vec![IntentCapability {
            family: IntentFamily::GimbalRate,
            frames: vec![],
            max_linear: 0.0,
            max_vertical: 0.0,
            max_angular: crate::gimbal::MAX_PITCH_RATE_RPS,
        }],
        actions: vec![ActionCapability {
            action: ActionKind::GimbalRecenter,
            mode_targets: vec![],
        }],
        legacy: Some(LegacyCommandMap::GimbalRate {
            pitch: Some(LegacyAxisRoute {
                axis: PITCH_AXIS,
                sign: 1.0,
            }),
            yaw: Some(LegacyAxisRoute {
                axis: YAW_AXIS,
                sign: 1.0,
            }),
            recenter_button: Some(GIMBAL_NEUTRAL_BUTTON),
        }),
    }
}
