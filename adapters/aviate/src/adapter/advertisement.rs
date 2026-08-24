//! The Aviate adapter's capability advertisement: the REAL flight envelope
//! the uplink enforces, the typed actions and mode targets the flight scope
//! consumes, and the legacy translation map the session host's compatibility
//! boundary interprets numeric payloads through (CTRL-01).

use pilotage_adapter_api::{
    ActionCapability, AdapterCapabilities, ControlFeelDescriptor, ControlFeelMode, ExecutionMode,
    IntentCapability, LegacyAxisRoute, LegacyCommandMap, LinkLossPolicy, ScopeDescriptor,
    VehicleDescriptor,
};
use pilotage_protocol::{ActionKind, IntentFamily, LogicalAxisId, ReferenceFrame, ScopeId};

#[cfg(feature = "sim")]
use super::pointing::MAX_PITCH_RATE_RPS;
use super::{
    ARM_BUTTON, AviateAdapter, DIRECT_SCOPE, DISARM_BUTTON, FLIGHT_SCOPE, PITCH_AXIS, ROLL_AXIS,
    THROTTLE_AXIS, YAW_AXIS,
};
#[cfg(feature = "sim")]
use super::{GIMBAL_NEUTRAL_BUTTON, GIMBAL_SCOPE};

impl AviateAdapter {
    /// The capability report `VehicleAdapter::capabilities` returns.
    pub(super) fn advertised_capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            execution: ExecutionMode {
                real_time: true,
                render_capable: self.camera_bridge.is_some(),
                ..ExecutionMode::default()
            },
            // Without a working velocity-control uplink, the adapter stays
            // telemetry-only as required by ADR-0018.
            vehicles: vec![VehicleDescriptor {
                id: self.vehicle,
                scopes: {
                    #[cfg_attr(not(feature = "sim"), allow(unused_mut))]
                    let mut scopes = if self.uplink.is_some() {
                        let envelope = self
                            .uplink
                            .as_ref()
                            .map(crate::uplink::FlightUplink::envelope);
                        let mut flight = envelope.map_or_else(Vec::new, |value| {
                            vec![
                                flight_scope_descriptor(value),
                                direct_scope_descriptor(value),
                            ]
                        });
                        // STRUCTURALLY simulation-only (SIM-01): the lifecycle
                        // scope exists only under the Simulation profile — a
                        // physical/RF vehicle neither advertises nor accepts
                        // it, uplink or no uplink.
                        if self.profile == super::AviateProfile::Simulation {
                            flight.push(pilotage_adapter_api::sim_lifecycle_descriptor());
                        }
                        flight
                    } else {
                        vec![]
                    };
                    // Pointing is independent of flight: a telemetry-only
                    // session still aims its payload view. The scope exists
                    // exactly while a producer accepts commands — never as
                    // an affordance with nothing behind it.
                    #[cfg(feature = "sim")]
                    if self.pointing.is_some() {
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
            adapter_version: adapter_version(self),
            control_feel: self.feel_entry().map(control_feel_descriptor),
        }
    }
}

fn adapter_version(_adapter: &AviateAdapter) -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

pub(super) fn control_feel_descriptor(
    entry: &super::control_feel::ControlFeelEntry,
) -> ControlFeelDescriptor {
    let bindings = entry.profile.profile().bindings;
    ControlFeelDescriptor {
        profile_id: entry.identity.profile_id.clone(),
        mode: control_feel_mode(entry.identity.mode),
        schema_version: entry.identity.schema,
        profile_sha256: *entry.identity.digest.as_bytes(),
        device_profile_sha256: *bindings.device_profile_sha256.as_bytes(),
        flight_controller_sha256: *bindings.flight_controller_sha256.as_bytes(),
    }
}

fn control_feel_mode(mode: pilotage_control_feel::FeelMode) -> ControlFeelMode {
    match mode {
        pilotage_control_feel::FeelMode::Precision => ControlFeelMode::Precision,
        pilotage_control_feel::FeelMode::Balanced => ControlFeelMode::Balanced,
        pilotage_control_feel::FeelMode::Agile => ControlFeelMode::Agile,
        pilotage_control_feel::FeelMode::LegacyCompatibility => {
            ControlFeelMode::LegacyCompatibility
        }
    }
}

/// Both flight scopes take the same discrete actions. There is NO mode
/// request (direct flight is its own scope with its own lease, never a mode
/// flip reinterpreting velocity numbers) and NO sim reset (a lifecycle
/// action under the separately leased `sim.lifecycle` scope, never flight
/// authority).
fn flight_actions() -> Vec<ActionCapability> {
    vec![
        ActionCapability {
            action: ActionKind::Arm,
            mode_targets: vec![],
        },
        ActionCapability {
            action: ActionKind::Disarm,
            mode_targets: vec![],
        },
    ]
}

/// The direct-flight scope: attitude + collective thrust, typed-only (no
/// legacy translation admits numeric payloads here). `max_angular` is the
/// tilt-angle bound the uplink clamps at; `max_yaw_rate` is the heading
/// slew the CLIENT integrates its yaw stick with, matching the velocity
/// scope's yaw envelope so direct flight turns no faster than camera
/// flight.
fn direct_scope_descriptor(envelope: pilotage_control_feel::DemandEnvelope) -> ScopeDescriptor {
    ScopeDescriptor {
        // One FC, one authority: velocity and direct flight can never be
        // held at once, and share generation, latch, and recovery.
        authority_group: Some(FLIGHT_SCOPE.to_owned()),
        scope: ScopeId::new(DIRECT_SCOPE),
        axes: vec![],
        intents: vec![IntentCapability {
            family: IntentFamily::AttitudeThrust,
            frames: vec![ReferenceFrame::LocalNed],
            max_linear: 0.0,
            max_vertical: 0.0,
            max_angular: envelope.direct_tilt_rad,
            max_yaw_rate: envelope.yaw_rate_rps,
        }],
        actions: flight_actions(),
        legacy: None,
    }
}

fn flight_scope_descriptor(envelope: pilotage_control_feel::DemandEnvelope) -> ScopeDescriptor {
    ScopeDescriptor {
        authority_group: Some(FLIGHT_SCOPE.to_owned()),
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
            max_yaw_rate: 0.0,
            family: IntentFamily::Velocity,
            frames: vec![ReferenceFrame::BodyFrd],
            max_linear: envelope.horizontal_speed_mps,
            max_vertical: envelope.vertical_speed_mps,
            max_angular: envelope.yaw_rate_rps,
        }],
        actions: flight_actions(),
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

/// The gimbal pointing scope. `max_angular` is the rate this adapter
/// actually integrates, so a client scales its normalized stick against
/// the enacted envelope rather than a borrowed constant.
#[cfg(feature = "sim")]
fn gimbal_scope_descriptor() -> ScopeDescriptor {
    ScopeDescriptor {
        // Its own authority group: pointing is leased and fenced apart
        // from flight, so losing one never disturbs the other.
        authority_group: None,
        scope: ScopeId::new(GIMBAL_SCOPE),
        axes: vec![LogicalAxisId::new(PITCH_AXIS), LogicalAxisId::new(YAW_AXIS)],
        intents: vec![IntentCapability {
            family: IntentFamily::GimbalRate,
            frames: vec![],
            max_linear: 0.0,
            max_vertical: 0.0,
            max_yaw_rate: 0.0,
            max_angular: MAX_PITCH_RATE_RPS,
        }],
        actions: vec![
            ActionCapability {
                action: ActionKind::GimbalRecenter,
                mode_targets: vec![],
            },
            // Zoom is DETENTED: each step selects a distinct camera model
            // whose calibration the frames carry (ADR-0021). A continuous
            // zoom would publish pictures with no resolvable intrinsics.
            ActionCapability {
                action: ActionKind::CameraZoomIn,
                mode_targets: vec![],
            },
            ActionCapability {
                action: ActionKind::CameraZoomOut,
                mode_targets: vec![],
            },
        ],
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
