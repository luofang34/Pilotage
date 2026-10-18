use super::VehicleClass;
use crate::wire;

const ALL: [VehicleClass; 5] = [
    VehicleClass::Multicopter,
    VehicleClass::FixedWing,
    VehicleClass::Vtol,
    VehicleClass::GroundRover,
    VehicleClass::SurfaceVessel,
];

#[test]
fn every_class_round_trips_through_the_wire() {
    for class in ALL {
        assert_eq!(VehicleClass::from_wire(class.to_wire() as i32), Some(class));
        assert_ne!(class.to_wire() as i32, 0, "{class:?} is not the default");
    }
}

#[test]
fn an_unspecified_or_unknown_class_is_none() {
    assert_eq!(
        VehicleClass::from_wire(wire::VehicleClass::Unspecified as i32),
        None
    );
    assert_eq!(VehicleClass::from_wire(42), None);
}
