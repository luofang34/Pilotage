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
