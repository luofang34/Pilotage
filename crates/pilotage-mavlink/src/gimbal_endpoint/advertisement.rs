use pilotage_adapter_api::{
    ActionCapability, IntentCapability, LegacyAxisRoute, LegacyCommandMap, ScopeDescriptor,
};
use pilotage_protocol::{ActionKind, IntentFamily, LogicalAxisId, ScopeId, wire};
use turret_core::{AxisCapabilities, Capability, Frames, ReferenceFrame};

use super::{ENDPOINT, GimbalStamp, NEUTRAL_BUTTON, PITCH_AXIS, SCOPE, YAW_AXIS};
use crate::ManagerControl;

/// Project discovered operations without granting aircraft motion authority.
#[must_use]
pub fn scope_descriptor(control: &ManagerControl, stamp: &GimbalStamp) -> ScopeDescriptor {
    let mut capabilities = control.capabilities();
    for axis in [
        &mut capabilities.gimbal.axes.pitch,
        &mut capabilities.gimbal.axes.roll,
        &mut capabilities.gimbal.axes.yaw,
    ] {
        apply_entrypoint_limits(axis);
    }
    let supports = |value: Capability<turret_core::RateCapability>| matches!(value, Capability::Available(rate) if rate.frames.contains(ReferenceFrame::HorizonVehicleYaw));
    let pitch = supports(capabilities.gimbal.axes.pitch.rate);
    let yaw = supports(capabilities.gimbal.axes.yaw.rate);
    let recenter = matches!(capabilities.gimbal.recenter, Capability::Available(_));
    let mut payload =
        pilotage_adapter_api::payload_capabilities_to_wire(ENDPOINT, &capabilities, &[SCOPE]);
    payload.connection_role = wire::PayloadConnectionRole::MavlinkManager as i32;
    payload.connection_incarnation = stamp.incarnation.into_bytes().to_vec();
    payload.connection_epoch = control.connection_epoch();
    ScopeDescriptor {
        payload: Some(payload),
        authority_group: None,
        scope: ScopeId::new(SCOPE),
        axes: [(PITCH_AXIS, pitch), (YAW_AXIS, yaw)]
            .into_iter()
            .filter_map(|(id, present)| present.then_some(LogicalAxisId::new(id)))
            .collect(),
        intents: if pitch || yaw {
            vec![IntentCapability {
                max_yaw_rate: 0.0,
                family: IntentFamily::GimbalRate,
                frames: vec![],
                max_linear: 0.0,
                max_vertical: 0.0,
                max_angular: crate::gimbal_control::MAX_PITCH_RATE_RPS,
            }]
        } else {
            vec![]
        },
        actions: if recenter {
            vec![ActionCapability {
                action: ActionKind::GimbalRecenter,
                mode_targets: vec![],
                feel_targets: vec![],
                takeoff_start_state: None,
            }]
        } else {
            vec![]
        },
        legacy: Some(LegacyCommandMap::GimbalRate {
            pitch: pitch.then_some(LegacyAxisRoute {
                axis: PITCH_AXIS,
                sign: 1.0,
            }),
            yaw: yaw.then_some(LegacyAxisRoute {
                axis: YAW_AXIS,
                sign: 1.0,
            }),
            recenter_button: recenter.then_some(NEUTRAL_BUTTON),
        }),
    }
}

fn apply_entrypoint_limits(axis: &mut AxisCapabilities) {
    // The adapter's typed frame carries rates and recenter only. A native
    // position command in the transport does not create a client command route.
    if let Capability::Available(capability) = axis.position {
        axis.position = Capability::Unimplemented {
            capability: Some(capability),
        };
    }
    if let Capability::Available(mut capability) = axis.rate {
        axis.rate = if capability.frames.horizon_vehicle_yaw {
            capability.frames = Frames {
                horizon_vehicle_yaw: true,
                ..Frames::default()
            };
            Capability::Available(capability)
        } else {
            Capability::Unimplemented {
                capability: Some(capability),
            }
        };
    }
}
