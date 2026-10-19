use super::GIMBAL_SCOPE;
use pilotage_adapter_api::{
    ActionCapability, AdapterCapabilities, ExecutionMode, IntentCapability, LinkLossPolicy,
    ScopeDescriptor, VehicleDescriptor,
};
use pilotage_protocol::{ActionKind, IntentFamily, ScopeId, VehicleClass, VehicleId};

pub(super) const MAX_PITCH_RATE: f32 = std::f32::consts::PI / 36.0;

pub(super) fn capabilities(vehicle: VehicleId) -> AdapterCapabilities {
    AdapterCapabilities {
        execution: ExecutionMode {
            real_time: true,
            physically_embodied: true,
            ..ExecutionMode::default()
        },
        vehicles: vec![VehicleDescriptor {
            id: vehicle,
            class: VehicleClass::Multicopter,
            heading_reference: None,
            display_name: "DJI Mini 3".to_owned(),
            scopes: vec![ScopeDescriptor {
                authority_group: None,
                scope: ScopeId::new(GIMBAL_SCOPE),
                axes: Vec::new(),
                intents: vec![IntentCapability {
                    family: IntentFamily::GimbalRate,
                    frames: Vec::new(),
                    max_linear: 0.0,
                    max_vertical: 0.0,
                    // GimbalRate uses one bound for both input axes. Control
                    // projects the demand onto pitch; yaw is not transmitted.
                    max_angular: MAX_PITCH_RATE,
                    max_yaw_rate: 0.0,
                }],
                actions: vec![ActionCapability {
                    action: ActionKind::GimbalRecenter,
                    mode_targets: Vec::new(),
                    feel_targets: Vec::new(),
                }],
                legacy: None,
            }],
            link_loss_actions: vec![LinkLossPolicy::Neutralize],
        }],
        adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
        control_feel: None,
    }
}
