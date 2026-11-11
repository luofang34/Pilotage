use super::{
    AltitudeReference, ControlAction, ControlIntent, ConvertError, GimbalRateIntent,
    HeadingAltitudeIntent, ReferenceFrame, VelocityIntent, action_from_wire, action_to_wire,
    intent_from_wire, intent_to_wire, wire,
};

#[test]
fn a_velocity_intent_round_trips_through_the_wire() {
    let intent = ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx: 1.5,
        vy: -0.5,
        vz: 0.0,
        yaw_rate: 0.25,
    });
    let round = intent_from_wire(intent_to_wire(&intent)).expect("round-trips");
    assert_eq!(round, intent);
}

#[test]
fn a_gimbal_rate_intent_round_trips_through_the_wire() {
    let intent = ControlIntent::GimbalRate(GimbalRateIntent {
        pitch_rate: -0.3,
        yaw_rate: 0.7,
    });
    assert_eq!(
        intent_from_wire(intent_to_wire(&intent)).expect("round-trips"),
        intent
    );
}

#[test]
fn an_absent_family_is_a_missing_field() {
    let empty = wire::ControlIntent { family: None };
    assert!(matches!(
        intent_from_wire(empty),
        Err(ConvertError::MissingField {
            field: "family",
            ..
        })
    ));
}

#[test]
fn a_non_finite_component_is_rejected() {
    let intent = ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx: f32::INFINITY,
        vy: 0.0,
        vz: 0.0,
        yaw_rate: 0.0,
    });
    assert!(matches!(
        intent_from_wire(intent_to_wire(&intent)),
        Err(ConvertError::NonFiniteIntentValue {
            field: "velocity.vx"
        })
    ));
}

#[test]
fn an_unspecified_frame_is_rejected() {
    let mut wire_intent = intent_to_wire(&ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx: 0.0,
        vy: 0.0,
        vz: 0.0,
        yaw_rate: 0.0,
    }));
    if let Some(wire::control_intent::Family::Velocity(ref mut v)) = wire_intent.family {
        v.frame = wire::ReferenceFrame::Unspecified as i32;
    }
    assert!(matches!(
        intent_from_wire(wire_intent),
        Err(ConvertError::UnknownEnum {
            enum_name: "pilotage.v1.ReferenceFrame",
            ..
        })
    ));
}

#[test]
fn actions_round_trip_and_reject_the_unspecified_sentinel() {
    for action in [
        ControlAction::Arm,
        ControlAction::Disarm,
        ControlAction::Takeoff,
        ControlAction::Land,
        ControlAction::ModeRequest {
            target: crate::intent::ModeTarget::Hold,
        },
        ControlAction::GimbalRecenter,
    ] {
        let request = action_to_wire(action, 7);
        let (round, id) = action_from_wire(request).expect("round-trips");
        assert_eq!(round, action);
        assert_eq!(id, 7, "the correlation id survives the wire");
    }
    assert!(matches!(
        action_from_wire(wire::ControlActionRequest {
            zoom: None,
            action: wire::ControlAction::Unspecified as i32,
            mode_target: 0,
            action_id: 0,
            feel_target: wire::FeelTarget::Unspecified as i32,
        }),
        Err(ConvertError::UnknownEnum {
            enum_name: "pilotage.v1.ControlAction",
            ..
        })
    ));
}

#[test]
fn a_mode_request_requires_an_explicit_target() {
    assert!(matches!(
        action_from_wire(wire::ControlActionRequest {
            zoom: None,
            action: wire::ControlAction::ModeRequest as i32,
            mode_target: wire::ModeTarget::Unspecified as i32,
            action_id: 0,
            feel_target: wire::FeelTarget::Unspecified as i32,
        }),
        Err(ConvertError::UnknownEnum {
            enum_name: "pilotage.v1.ModeTarget",
            ..
        })
    ));
}

#[test]
fn a_stray_mode_target_on_a_non_mode_action_is_rejected() {
    for action in [
        wire::ControlAction::Arm,
        wire::ControlAction::Takeoff,
        wire::ControlAction::Land,
    ] {
        assert!(matches!(
            action_from_wire(wire::ControlActionRequest {
                zoom: None,
                action: action as i32,
                mode_target: wire::ModeTarget::Hold as i32,
                action_id: 0,
                feel_target: wire::FeelTarget::Unspecified as i32,
            }),
            Err(ConvertError::UnknownEnum {
                enum_name: "pilotage.v1.ControlActionRequest.mode_target",
                ..
            })
        ));
    }
}

#[test]
fn a_non_unit_quaternion_is_rejected() {
    let wire_intent = wire::ControlIntent {
        family: Some(wire::control_intent::Family::AttitudeThrust(
            wire::AttitudeThrustIntent {
                frame: wire::ReferenceFrame::LocalNed as i32,
                qw: 0.5,
                qx: 0.0,
                qy: 0.0,
                qz: 0.0,
                thrust: 0.5,
            },
        )),
    };
    assert!(matches!(
        intent_from_wire(wire_intent),
        Err(ConvertError::InvalidQuaternion { .. })
    ));
}

#[test]
fn a_thrust_outside_unit_range_is_rejected() {
    let wire_intent = wire::ControlIntent {
        family: Some(wire::control_intent::Family::BodyRate(
            wire::BodyRateIntent {
                roll_rate: 0.0,
                pitch_rate: 0.0,
                yaw_rate: 0.0,
                thrust: 1.5,
            },
        )),
    };
    assert!(matches!(
        intent_from_wire(wire_intent),
        Err(ConvertError::IntentOutOfRange {
            field: "body_rate.thrust",
            ..
        })
    ));
}

fn heading_altitude() -> HeadingAltitudeIntent {
    HeadingAltitudeIntent {
        heading_rad: -1.25,
        altitude_m: 812.5,
        reference: AltitudeReference::Ellipsoid,
        airspeed_mps: Some(52.0),
    }
}

fn heading_altitude_wire(
    change: impl FnOnce(&mut wire::HeadingAltitudeIntent),
) -> wire::ControlIntent {
    let mut wire = intent_to_wire(&ControlIntent::HeadingAltitude(heading_altitude()));
    let Some(wire::control_intent::Family::HeadingAltitude(intent)) = wire.family.as_mut() else {
        panic!("the heading and altitude family encodes as itself");
    };
    change(intent);
    wire
}

#[test]
fn a_heading_altitude_intent_round_trips_with_each_reference_and_airspeed_state() {
    for reference in [AltitudeReference::Ellipsoid, AltitudeReference::LocalOrigin] {
        for airspeed_mps in [Some(52.0), None] {
            let intent = ControlIntent::HeadingAltitude(HeadingAltitudeIntent {
                reference,
                airspeed_mps,
                ..heading_altitude()
            });
            assert_eq!(
                intent_from_wire(intent_to_wire(&intent)).expect("round-trips"),
                intent
            );
        }
    }
}

#[test]
fn the_altitude_reference_codes_are_the_vertical_datum_codes_of_a_fix() {
    // pilotage_geo::VerticalDatum: 1 is the ellipsoid, 6 is local-relative.
    assert_eq!(AltitudeReference::Ellipsoid.code(), 1);
    assert_eq!(AltitudeReference::LocalOrigin.code(), 6);
    for code in [0, 2, 3, 4, 5, 7, u32::MAX] {
        assert_eq!(AltitudeReference::from_code(code), None);
        assert!(matches!(
            intent_from_wire(heading_altitude_wire(|intent| intent.vertical_datum = code)),
            Err(ConvertError::UnknownEnum {
                enum_name: "pilotage.v1.HeadingAltitudeIntent.vertical_datum",
                ..
            })
        ));
    }
}

#[test]
fn a_heading_outside_one_turn_and_a_non_finite_value_are_refused() {
    assert!(matches!(
        intent_from_wire(heading_altitude_wire(|intent| intent.heading_rad = 3.2)),
        Err(ConvertError::IntentOutOfRange {
            field: "heading_altitude.heading_rad",
            ..
        })
    ));
    assert!(matches!(
        intent_from_wire(heading_altitude_wire(|intent| intent.heading_rad = f32::NAN)),
        Err(ConvertError::NonFiniteIntentValue {
            field: "heading_altitude.heading_rad"
        })
    ));
    assert!(matches!(
        intent_from_wire(heading_altitude_wire(
            |intent| intent.altitude_m = f32::INFINITY
        )),
        Err(ConvertError::NonFiniteIntentValue {
            field: "heading_altitude.altitude_m"
        })
    ));
}

#[test]
fn an_airspeed_that_is_not_greater_than_zero_is_refused_and_absence_is_not_zero() {
    for airspeed in [0.0, -3.0] {
        assert!(matches!(
            intent_from_wire(heading_altitude_wire(|intent| {
                intent.airspeed_mps = Some(airspeed);
            })),
            Err(ConvertError::IntentOutOfRange {
                field: "heading_altitude.airspeed_mps",
                ..
            })
        ));
    }
    assert!(matches!(
        intent_from_wire(heading_altitude_wire(|intent| {
            intent.airspeed_mps = Some(f32::NAN);
        })),
        Err(ConvertError::NonFiniteIntentValue {
            field: "heading_altitude.airspeed_mps"
        })
    ));
    let absent = heading_altitude_wire(|intent| intent.airspeed_mps = None);
    let ControlIntent::HeadingAltitude(intent) = intent_from_wire(absent).expect("decodes") else {
        panic!("the family decodes as itself");
    };
    assert_eq!(intent.airspeed_mps, None);
}
