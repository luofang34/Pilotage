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
    Px4Adapter, ROLL_AXIS, THROTTLE_AXIS, YAW_AXIS,
};

/// The velocity envelope the flight scope advertises and enforces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FlightEnvelope {
    /// Horizontal bound, m/s.
    pub(super) max_linear: f32,
    /// Vertical bound, m/s.
    pub(super) max_vertical: f32,
    /// Yaw-rate bound, rad/s.
    pub(super) max_angular: f32,
}

impl Px4Adapter {
    /// The REAL envelope of the airframe's uplink. A multicopter flies the
    /// offboard velocity envelope. A fixed wing reads `max_linear` as the
    /// airspeed change from trim (the same bound below and above trim),
    /// `max_vertical` as the climb or sink rate, and `max_angular` as the
    /// coordinated-turn rate at trim airspeed; it flies no lateral `vy`.
    pub(super) fn flight_envelope(&self) -> FlightEnvelope {
        if self.fixed_wing.is_some() {
            FlightEnvelope {
                max_linear: crate::fixed_wing::sticks::MAX_AIRSPEED_CHANGE_MPS,
                max_vertical: crate::fixed_wing::sticks::MAX_VERTICAL_MPS,
                max_angular: crate::fixed_wing::sticks::max_turn_rate_rps(),
            }
        } else {
            FlightEnvelope {
                max_linear: crate::uplink::MAX_HORIZONTAL_MPS,
                max_vertical: crate::uplink::MAX_VERTICAL_MPS,
                max_angular: crate::uplink::MAX_YAW_RATE_RPS,
            }
        }
    }

    fn has_flight_uplink(&self) -> bool {
        self.uplink.is_some() || self.fixed_wing.is_some()
    }

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
                    if self.has_flight_uplink() {
                        scopes.push(flight_scope_descriptor(
                            self.flight_envelope(),
                            self.fixed_wing.is_none(),
                        ));
                        // This adapter implements only the SIMULATION
                        // profile today; a physical-vehicle profile must
                        // NOT advertise the lifecycle scope (SIM-01).
                        scopes.push(pilotage_adapter_api::sim_lifecycle_descriptor());
                    }
                    if self.gimbal.is_some() {
                        scopes.push(gimbal_scope_descriptor());
                    }
                    scopes
                },
                link_loss_actions: if self.has_flight_uplink() {
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

/// `lateral` is false for an airframe that cannot fly `vy`: its legacy map
/// routes no axis there.
fn flight_scope_descriptor(envelope: FlightEnvelope, lateral: bool) -> ScopeDescriptor {
    ScopeDescriptor {
        authority_group: None,
        scope: ScopeId::new(FLIGHT_SCOPE),
        axes: vec![
            LogicalAxisId::new(ROLL_AXIS),
            LogicalAxisId::new(PITCH_AXIS),
            LogicalAxisId::new(THROTTLE_AXIS),
            LogicalAxisId::new(YAW_AXIS),
        ],
        // The REAL flight envelope the uplink enforces.
        intents: vec![IntentCapability {
            max_yaw_rate: 0.0,
            family: IntentFamily::Velocity,
            frames: vec![ReferenceFrame::BodyFrd],
            max_linear: envelope.max_linear,
            max_vertical: envelope.max_vertical,
            max_angular: envelope.max_angular,
        }],
        // No sim reset here: a lifecycle action under the separately
        // leased `sim.lifecycle` scope, never flight authority.
        actions: vec![
            ActionCapability {
                action: ActionKind::Arm,
                mode_targets: vec![],
                feel_targets: vec![],
            },
            ActionCapability {
                action: ActionKind::Disarm,
                mode_targets: vec![],
                feel_targets: vec![],
            },
        ],
        legacy: Some(LegacyCommandMap::Velocity {
            vx: Some(LegacyAxisRoute {
                axis: PITCH_AXIS,
                sign: 1.0,
            }),
            vy: lateral.then_some(LegacyAxisRoute {
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

fn gimbal_scope_descriptor() -> ScopeDescriptor {
    ScopeDescriptor {
        authority_group: None,
        scope: ScopeId::new(GIMBAL_SCOPE),
        axes: vec![LogicalAxisId::new(PITCH_AXIS), LogicalAxisId::new(YAW_AXIS)],
        intents: vec![IntentCapability {
            max_yaw_rate: 0.0,
            family: IntentFamily::GimbalRate,
            frames: vec![],
            max_linear: 0.0,
            max_vertical: 0.0,
            max_angular: crate::gimbal::MAX_PITCH_RATE_RPS,
        }],
        actions: vec![ActionCapability {
            action: ActionKind::GimbalRecenter,
            mode_targets: vec![],
            feel_targets: vec![],
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
