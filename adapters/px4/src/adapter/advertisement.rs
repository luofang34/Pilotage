//! The PX4 adapter's capability advertisement: the REAL flight and gimbal
//! envelopes the uplink and gimbal command paths enforce, the typed actions
//! each scope consumes, and the legacy translation maps the session host's
//! compatibility boundary interprets numeric payloads through (CTRL-01).

use pilotage_adapter_api::{
    ActionCapability, AdapterCapabilities, ExecutionMode, IntentCapability, LegacyAxisRoute,
    LegacyCommandMap, LinkLossPolicy, ScopeDescriptor, VehicleDescriptor,
};
use pilotage_protocol::{
    ActionKind, IntentFamily, LogicalAxisId, ReferenceFrame, ScopeId, VehicleClass,
};

use super::{
    ARM_BUTTON, DIRECT_SCOPE, DISARM_BUTTON, FLIGHT_SCOPE, GIMBAL_NEUTRAL_BUTTON, GIMBAL_SCOPE,
    PITCH_AXIS, Px4Adapter, ROLL_AXIS, THROTTLE_AXIS, YAW_AXIS,
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
                class: self.vehicle_class,
                display_name: display_name(self.vehicle_class),
                scopes: {
                    let mut scopes = Vec::new();
                    if self.has_flight_uplink() {
                        scopes.push(flight_scope_descriptor(
                            self.flight_envelope(),
                            self.fixed_wing.is_none(),
                        ));
                        // Direct flight is multicopter only: a fixed wing would
                        // have to leave Position mode for ACRO or STABILIZED,
                        // where PX4 passes the throttle stick straight through
                        // with no TECS airspeed floor, so the adapter could not
                        // bound a stall, and neutral sticks would no longer hold.
                        if self.uplink.is_some() {
                            scopes.push(direct_scope_descriptor());
                        }
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

/// The operator's name for a PX4 vehicle of this class.
fn display_name(class: VehicleClass) -> String {
    match class {
        VehicleClass::Multicopter => "PX4 multicopter",
        VehicleClass::FixedWing => "PX4 fixed-wing",
        VehicleClass::Vtol => "PX4 VTOL",
        VehicleClass::GroundRover => "PX4 rover",
        VehicleClass::SurfaceVessel => "PX4 boat",
    }
    .to_owned()
}

/// `lateral` is false for an airframe that cannot fly `vy`: its legacy map
/// routes no axis there.
fn flight_scope_descriptor(envelope: FlightEnvelope, lateral: bool) -> ScopeDescriptor {
    ScopeDescriptor {
        // One FC, one authority: velocity and direct flight are never held
        // at once. A fixed wing has no direct scope; the group is harmless.
        authority_group: Some(FLIGHT_SCOPE.to_owned()),
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
        actions: flight_actions(),
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

/// Both flight scopes take the same discrete actions; no sim reset (that
/// lives on the separately leased `sim.lifecycle` scope).
fn flight_actions() -> Vec<ActionCapability> {
    [ActionKind::Arm, ActionKind::Disarm]
        .into_iter()
        .map(|action| ActionCapability {
            action,
            mode_targets: vec![],
            feel_targets: vec![],
        })
        .collect()
}

/// The multicopter direct-flight scope, typed only (no legacy map admits
/// numeric payloads here): body rates or an attitude, each with collective
/// thrust. The thrust is held at PX4's own commanded thrust when direct
/// flight starts and passes to the operator once their thrust reaches it
/// (`uplink/direct.rs`). `max_yaw_rate` is the heading slew a client
/// integrates its yaw stick with for attitude flight, the velocity scope's
/// yaw envelope.
fn direct_scope_descriptor() -> ScopeDescriptor {
    ScopeDescriptor {
        authority_group: Some(FLIGHT_SCOPE.to_owned()),
        scope: ScopeId::new(DIRECT_SCOPE),
        axes: vec![],
        intents: vec![
            IntentCapability {
                family: IntentFamily::BodyRate,
                frames: vec![],
                max_linear: 0.0,
                max_vertical: 0.0,
                max_angular: crate::uplink::direct::MAX_BODY_RATE_RPS,
                max_yaw_rate: 0.0,
            },
            IntentCapability {
                family: IntentFamily::AttitudeThrust,
                frames: vec![ReferenceFrame::LocalNed],
                max_linear: 0.0,
                max_vertical: 0.0,
                max_angular: crate::uplink::direct::MAX_TILT_RAD,
                max_yaw_rate: crate::uplink::MAX_YAW_RATE_RPS,
            },
        ],
        actions: flight_actions(),
        legacy: None,
    }
}
