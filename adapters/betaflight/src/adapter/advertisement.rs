//! The capability advertisement: the assisted scope taking BODY_FRD
//! velocity inside the envelope the outer loop flies, and the direct scope
//! taking body rates or an attitude with thrust inside Betaflight's own
//! rate and angle limits. Both carry ARM and DISARM and share one
//! authority group: one Betaflight, one pilot.

use pilotage_adapter_api::{
    ActionCapability, AdapterCapabilities, ExecutionMode, IntentCapability, LegacyAxisRoute,
    LegacyCommandMap, LinkLossPolicy, ScopeDescriptor, VehicleDescriptor,
};
use pilotage_protocol::{
    ActionKind, HeadingReference, IntentFamily, LogicalAxisId, ReferenceFrame, ScopeId,
    VehicleClass,
};

use super::{
    ARM_BUTTON, BetaflightAdapter, DIRECT_SCOPE, DISARM_BUTTON, FLIGHT_SCOPE, PITCH_AXIS,
    ROLL_AXIS, THROTTLE_AXIS, YAW_AXIS,
};
use crate::config::{
    MAX_BODY_RATE_RPS, MAX_HORIZONTAL_MPS, MAX_TILT_RAD, MAX_VERTICAL_MPS, MAX_YAW_RATE_RPS,
};

/// The operator's name for the vehicle.
const DISPLAY_NAME: &str = "Betaflight quad";

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
                class: VehicleClass::Multicopter,
                display_name: DISPLAY_NAME.to_owned(),
                scopes: vec![flight_scope_descriptor(), direct_scope_descriptor()],
                link_loss_actions: vec![LinkLossPolicy::Neutralize],
                // Betaflight applies no declination: its yaw is magnetic.
                heading_reference: Some(HeadingReference::Magnetic),
            }],
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
            control_feel: None,
        }
    }
}

fn flight_actions() -> Vec<ActionCapability> {
    [ActionKind::Arm, ActionKind::Disarm]
        .map(|action| ActionCapability {
            action,
            mode_targets: vec![],
            feel_targets: vec![],
        })
        .to_vec()
}

/// The direct scope, typed only (no legacy map admits numeric payloads
/// here). Body rates fly ACRO up to Betaflight's full-stick rate on every
/// axis; an attitude flies ANGLE up to its angle limit, and the heading
/// part slews no faster than the assisted scope's yaw envelope.
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
                max_angular: MAX_BODY_RATE_RPS,
                max_yaw_rate: 0.0,
            },
            IntentCapability {
                family: IntentFamily::AttitudeThrust,
                frames: vec![ReferenceFrame::LocalNed],
                max_linear: 0.0,
                max_vertical: 0.0,
                max_angular: MAX_TILT_RAD,
                max_yaw_rate: MAX_YAW_RATE_RPS,
            },
        ],
        actions: flight_actions(),
        legacy: None,
    }
}

fn flight_scope_descriptor() -> ScopeDescriptor {
    ScopeDescriptor {
        // One FC, one authority: assisted and direct flight are never held
        // at once.
        authority_group: Some(FLIGHT_SCOPE.to_owned()),
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
