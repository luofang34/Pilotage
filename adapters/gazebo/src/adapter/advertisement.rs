//! The Gazebo rover adapter's capability advertisement: the unit twist
//! envelope the bridge forwards, and the legacy translation map the session
//! host's compatibility boundary interprets numeric payloads through
//! (CTRL-01).

use pilotage_adapter_api::{
    AdapterCapabilities, ExecutionMode, IntentCapability, LegacyAxisRoute, LegacyCommandMap,
    LinkLossPolicy, ScopeDescriptor, VehicleDescriptor,
};
use pilotage_protocol::{IntentFamily, LogicalAxisId, ReferenceFrame, ScopeId};

use super::{
    GazeboAdapter, MAX_ANGULAR_RPS, MAX_LINEAR_MPS, MOTION_SCOPE, THROTTLE_AXIS, YAW_AXIS,
};

impl GazeboAdapter {
    /// The capability report `VehicleAdapter::capabilities` returns.
    pub(super) fn advertised_capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            execution: ExecutionMode {
                real_time: true,
                render_capable: true,
                physically_embodied: false,
                ..ExecutionMode::default()
            },
            vehicles: vec![VehicleDescriptor {
                id: self.vehicle,
                scopes: vec![ScopeDescriptor {
                    scope: ScopeId::new(MOTION_SCOPE),
                    axes: vec![
                        LogicalAxisId::new(THROTTLE_AXIS),
                        LogicalAxisId::new(YAW_AXIS),
                    ],
                    // The bridge forwards linear.x / angular.z at unit scale,
                    // so the advertised envelope is the unit twist bound.
                    intents: vec![IntentCapability {
                        family: IntentFamily::Velocity,
                        frames: vec![ReferenceFrame::BodyFrd],
                        max_linear: MAX_LINEAR_MPS,
                        max_vertical: 0.0,
                        max_angular: MAX_ANGULAR_RPS,
                    }],
                    actions: vec![],
                    legacy: Some(LegacyCommandMap::Velocity {
                        vx: Some(LegacyAxisRoute {
                            axis: THROTTLE_AXIS,
                            sign: 1.0,
                        }),
                        vy: None,
                        vz: None,
                        yaw_rate: Some(LegacyAxisRoute {
                            axis: YAW_AXIS,
                            sign: 1.0,
                        }),
                        arm_button: None,
                        disarm_button: None,
                        reset_button: None,
                    }),
                }],
                link_loss_actions: vec![LinkLossPolicy::Neutralize],
            }],
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}
