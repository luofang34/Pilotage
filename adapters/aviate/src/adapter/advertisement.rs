//! The Aviate adapter's capability advertisement: the REAL flight envelope
//! the uplink enforces, the typed actions and mode targets the flight scope
//! consumes, and the legacy translation map the session host's compatibility
//! boundary interprets numeric payloads through (CTRL-01).

use pilotage_adapter_api::{
    ActionCapability, AdapterCapabilities, ExecutionMode, IntentCapability, LegacyAxisRoute,
    LegacyCommandMap, LinkLossPolicy, ScopeDescriptor, VehicleDescriptor,
};
use pilotage_protocol::{
    ActionKind, IntentFamily, LogicalAxisId, ModeTarget, ReferenceFrame, ScopeId,
};

use super::{
    ARM_BUTTON, AviateAdapter, DISARM_BUTTON, FLIGHT_SCOPE, PITCH_AXIS, RESET_BUTTON, ROLL_AXIS,
    THROTTLE_AXIS, YAW_AXIS,
};

impl AviateAdapter {
    /// The capability report `VehicleAdapter::capabilities` returns.
    pub(super) fn advertised_capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            execution: ExecutionMode {
                real_time: true,
                render_capable: self._camera_bridge.is_some(),
                ..ExecutionMode::default()
            },
            // Without a working velocity-control uplink, the adapter stays
            // telemetry-only as required by ADR-0018.
            vehicles: vec![VehicleDescriptor {
                id: self.vehicle,
                scopes: if self.uplink.is_some() {
                    vec![flight_scope_descriptor()]
                } else {
                    vec![]
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
        // The REAL flight envelope the uplink enforces: a
        // client scaling sticks by these limits commands
        // exactly what full stick flies today.
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
            ActionCapability {
                action: ActionKind::ModeRequest,
                mode_targets: vec![ModeTarget::CameraVelocity, ModeTarget::FpvDirect],
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
