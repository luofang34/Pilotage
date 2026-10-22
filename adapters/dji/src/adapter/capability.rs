use super::{DjiControlMode, GIMBAL_SCOPE};
use pilotage_adapter_api::{
    ActionCapability, AdapterCapabilities, ExecutionMode, IntentCapability, LinkLossPolicy,
    ScopeDescriptor, VehicleDescriptor,
};
use pilotage_protocol::{ActionKind, IntentFamily, ScopeId, VehicleClass, VehicleId};

pub(super) fn set_availability(
    contract: &mut turret_core::PayloadCapabilities,
    reason: Option<turret_core::UnavailableReason>,
) {
    contract.gimbal.axes.pitch.position = availability(contract.gimbal.axes.pitch.position, reason);
    contract.gimbal.axes.pitch.rate = availability(contract.gimbal.axes.pitch.rate, reason);
    contract.gimbal.recenter = availability(contract.gimbal.recenter, reason);
}

fn availability<T>(
    value: turret_core::Capability<T>,
    reason: Option<turret_core::UnavailableReason>,
) -> turret_core::Capability<T> {
    use turret_core::Capability;
    match value {
        Capability::Available(capability) | Capability::Unavailable { capability, .. } => {
            match reason {
                Some(reason) => Capability::Unavailable { capability, reason },
                None => Capability::Available(capability),
            }
        }
        other => other,
    }
}

pub(super) fn capabilities(
    vehicle: VehicleId,
    mode: DjiControlMode,
    profile: turret_dji::Mini3ControlProfile,
    payload: &turret_core::PayloadCapabilities,
    incarnation: pilotage_adapter_api::SourceIncarnation,
    experimental_flight: bool,
    experimental_virtual_stick: bool,
) -> AdapterCapabilities {
    let mut payload =
        pilotage_adapter_api::payload_capabilities_to_wire("dji-camera", payload, &[GIMBAL_SCOPE]);
    payload.connection_incarnation = incarnation.into_bytes().to_vec();
    payload.connection_epoch = 0;
    payload.connection_role = pilotage_protocol::wire::PayloadConnectionRole::DirectDevice as i32;
    let mut scopes = vec![gimbal_scope(payload, profile)];
    if experimental_flight || experimental_virtual_stick {
        scopes.push(flight_scope(
            experimental_flight,
            experimental_virtual_stick,
        ));
    }
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
            display_name: match mode {
                DjiControlMode::Observed => "DJI Mini 3",
                DjiControlMode::BenchInitialHorizontal => "DJI Mini 3 · one-way bench",
            }
            .to_owned(),
            scopes,
            link_loss_actions: match mode {
                DjiControlMode::Observed => vec![LinkLossPolicy::Neutralize],
                DjiControlMode::BenchInitialHorizontal => Vec::new(),
            },
        }],
        adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
        control_feel: None,
    }
}

fn gimbal_scope(
    payload: pilotage_protocol::wire::PayloadCapabilities,
    profile: turret_dji::Mini3ControlProfile,
) -> ScopeDescriptor {
    ScopeDescriptor {
        payload: Some(payload),
        authority_group: None,
        scope: ScopeId::new(GIMBAL_SCOPE),
        axes: Vec::new(),
        intents: vec![IntentCapability {
            family: IntentFamily::GimbalRate,
            frames: Vec::new(),
            max_linear: 0.0,
            max_vertical: 0.0,
            max_angular: profile.rate_limit().as_radians_per_second(),
            max_yaw_rate: 0.0,
        }],
        actions: vec![ActionCapability {
            action: ActionKind::GimbalRecenter,
            mode_targets: Vec::new(),
            feel_targets: Vec::new(),
            takeoff_start_state: None,
        }],
        legacy: None,
    }
}

fn flight_scope(actions: bool, continuous: bool) -> ScopeDescriptor {
    ScopeDescriptor {
        payload: None,
        authority_group: None,
        scope: ScopeId::new(super::FLIGHT_SCOPE),
        axes: Vec::new(),
        // The DJI virtual-stick protocol control range: the
        // adapter adds no limit of its own.
        intents: if continuous {
            vec![IntentCapability {
                family: IntentFamily::Velocity,
                frames: vec![pilotage_protocol::ReferenceFrame::BodyFrd],
                max_linear: crate::flight::HORIZONTAL_LIMIT_MPS,
                max_vertical: crate::flight::VERTICAL_LIMIT_MPS,
                max_angular: crate::flight::YAW_RATE_LIMIT_RPS,
                max_yaw_rate: 0.0,
            }]
        } else {
            Vec::new()
        },
        actions: [ActionKind::Takeoff, ActionKind::Land]
            .into_iter()
            .filter(|_| actions)
            .map(|action| ActionCapability {
                action,
                mode_targets: Vec::new(),
                feel_targets: Vec::new(),
                takeoff_start_state: (action == ActionKind::Takeoff)
                    .then_some(pilotage_protocol::TakeoffStartState::MotorsStopped),
            })
            .collect(),
        legacy: None,
    }
}
