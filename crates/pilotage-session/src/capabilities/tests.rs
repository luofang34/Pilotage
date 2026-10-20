use pilotage_adapter_api::{AdapterCapabilities, ExecutionMode, VehicleDescriptor};
use pilotage_protocol::{VehicleClass, VehicleId};

use super::host_capabilities;

fn caps(class: VehicleClass, name: &str, physically_embodied: bool) -> AdapterCapabilities {
    AdapterCapabilities {
        execution: ExecutionMode {
            real_time: true,
            physically_embodied,
            ..ExecutionMode::default()
        },
        vehicles: vec![VehicleDescriptor {
            id: VehicleId::new(3),
            class,
            display_name: name.to_owned(),
            scopes: vec![],
            link_loss_actions: vec![],
            heading_reference: None,
        }],
        adapter_version: "0.1.0".to_owned(),
        control_feel: None,
    }
}

#[test]
fn the_vehicle_class_and_name_reach_the_wire_unchanged() {
    for class in [
        VehicleClass::Multicopter,
        VehicleClass::FixedWing,
        VehicleClass::Vtol,
        VehicleClass::GroundRover,
        VehicleClass::SurfaceVessel,
    ] {
        let wire = host_capabilities(&caps(class, "PX4 fixed-wing", false), "1.0");
        let vehicle = &wire.vehicles[0];
        assert_eq!(VehicleClass::from_wire(vehicle.vehicle_class), Some(class));
        assert_eq!(vehicle.display_name, "PX4 fixed-wing");
    }
}

#[test]
fn the_heading_reference_reaches_the_wire_and_unknown_is_unspecified() {
    use pilotage_protocol::HeadingReference;
    for reference in [
        Some(HeadingReference::True),
        Some(HeadingReference::Magnetic),
        None,
    ] {
        let mut adapter = caps(VehicleClass::Multicopter, "quad", false);
        adapter.vehicles[0].heading_reference = reference;
        let wire = host_capabilities(&adapter, "1.0");
        assert_eq!(
            HeadingReference::from_wire(wire.vehicles[0].heading_reference),
            reference
        );
    }
}

#[test]
fn physical_embodiment_reaches_the_wire() {
    let simulated = host_capabilities(&caps(VehicleClass::Multicopter, "sim", false), "1.0");
    assert!(!simulated.physically_embodied);
    let physical = host_capabilities(&caps(VehicleClass::Multicopter, "real", true), "1.0");
    assert!(physical.physically_embodied);
}

#[test]
fn takeoff_is_advertised_only_as_its_own_action() {
    use pilotage_adapter_api::{ActionCapability, ScopeDescriptor};
    use pilotage_protocol::{ActionKind, ScopeId};
    let mut adapter = caps(VehicleClass::Multicopter, "quad", false);
    adapter.vehicles[0].scopes.push(ScopeDescriptor {
        payload: None,
        authority_group: None,
        scope: ScopeId::new("vehicle.motion"),
        axes: vec![],
        intents: vec![],
        actions: vec![ActionCapability {
            action: ActionKind::Takeoff,
            mode_targets: vec![],
            feel_targets: vec![],
            takeoff_start_state: None,
        }],
        legacy: None,
    });
    let wire = host_capabilities(&adapter, "1.0");
    let actions = &wire.vehicles[0].scopes[0].actions;
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0].action,
        pilotage_protocol::wire::ControlAction::Takeoff as i32
    );
    assert!(actions[0].mode_targets.is_empty());
}

#[test]
fn land_is_advertised_without_takeoff_or_disarm_support() {
    use pilotage_adapter_api::{ActionCapability, ScopeDescriptor};
    use pilotage_protocol::{ActionKind, ScopeId};
    let mut adapter = caps(VehicleClass::Multicopter, "quad", true);
    adapter.vehicles[0].scopes.push(ScopeDescriptor {
        payload: None,
        authority_group: None,
        scope: ScopeId::new("vehicle.motion"),
        axes: vec![],
        intents: vec![],
        actions: vec![ActionCapability {
            action: ActionKind::Land,
            mode_targets: vec![],
            feel_targets: vec![],
            takeoff_start_state: None,
        }],
        legacy: None,
    });
    let wire = host_capabilities(&adapter, "1.0");
    let scope = &wire.vehicles[0].scopes[0];
    assert_eq!(scope.actions.len(), 1);
    assert_eq!(
        scope.actions[0].action,
        pilotage_protocol::wire::ControlAction::Land as i32
    );
    assert_eq!(scope.actions[0].takeoff_start_state, 0);
    assert!(scope.intents.is_empty());
    assert!(scope.actions[0].mode_targets.is_empty());
}

#[test]
fn each_scope_carries_its_authority_group_to_the_wire() {
    use pilotage_adapter_api::ScopeDescriptor;
    use pilotage_protocol::ScopeId;
    let scope = |name: &str, group: Option<&str>| ScopeDescriptor {
        payload: None,
        authority_group: group.map(str::to_owned),
        scope: ScopeId::new(name),
        axes: vec![],
        intents: vec![],
        actions: vec![],
        legacy: None,
    };
    let mut adapter = caps(VehicleClass::Multicopter, "quad", false);
    adapter.vehicles[0].scopes = vec![
        scope("vehicle.motion", Some("vehicle.motion")),
        scope("vehicle.motion.direct", Some("vehicle.motion")),
        scope("vehicle.gimbal", None),
    ];
    let wire = host_capabilities(&adapter, "1.0");
    let groups: Vec<&str> = wire.vehicles[0]
        .scopes
        .iter()
        .map(|scope| scope.authority_group.as_str())
        .collect();
    assert_eq!(groups, vec!["vehicle.motion", "vehicle.motion", ""]);
}

#[test]
fn command_only_applies_to_the_whole_group_and_checks_each_input_form() {
    use pilotage_adapter_api::{LegacyCommandMap, ScopeDescriptor};
    use pilotage_protocol::{LogicalAxisId, ScopeId};
    let scope = |name: &str, group: Option<&str>| ScopeDescriptor {
        scope: ScopeId::new(name),
        authority_group: group.map(str::to_owned),
        payload: None,
        axes: vec![],
        intents: vec![],
        actions: vec![],
        legacy: None,
    };
    let mut adapter = caps(VehicleClass::Multicopter, "camera aircraft", true);
    adapter.vehicles[0].scopes = vec![
        scope("vehicle.motion", None),
        scope("vehicle.motion.direct", Some("vehicle.motion")),
        scope("vehicle.camera", None),
    ];
    assert!(!adapter.vehicles[0].is_command_only_group(&ScopeId::new("unknown")));
    assert!(
        host_capabilities(&adapter, "test").vehicles[0]
            .scopes
            .iter()
            .all(|s| s.command_only)
    );
    for form in 0..3 {
        let mut mixed = adapter.clone();
        let member = &mut mixed.vehicles[0].scopes[1];
        match form {
            0 => member.axes.push(LogicalAxisId::new(1)),
            1 => {
                member.legacy = Some(LegacyCommandMap::GimbalRate {
                    pitch: None,
                    yaw: None,
                    recenter_button: None,
                })
            }
            _ => member.intents.push(pilotage_adapter_api::IntentCapability {
                family: pilotage_protocol::IntentFamily::Velocity,
                frames: vec![],
                max_linear: 1.0,
                max_vertical: 1.0,
                max_angular: 1.0,
                max_yaw_rate: 1.0,
            }),
        }
        for reverse in [false, true] {
            if reverse {
                mixed.vehicles[0].scopes.reverse();
            }
            let wire = host_capabilities(&mixed, "test");
            for descriptor in &wire.vehicles[0].scopes {
                let camera = descriptor.scope.as_ref().unwrap().value == "vehicle.camera";
                assert_eq!(descriptor.command_only, camera);
            }
        }
    }
}
