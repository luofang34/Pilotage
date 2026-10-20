use super::{
    ControlAction, ControlIntent, ConvertError, GimbalRateIntent, ReferenceFrame, VelocityIntent,
    action_from_wire, action_to_wire, intent_from_wire, intent_to_wire, wire,
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
    for action in [wire::ControlAction::Arm, wire::ControlAction::Takeoff] {
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
