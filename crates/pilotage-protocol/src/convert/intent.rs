//! `ControlIntent` / `ControlAction` wire ↔ domain conversions (CTRL-01).
//!
//! Domain-to-wire is infallible. Wire-to-domain rejects an absent `oneof`
//! family, an unknown or unspecified reference frame or action, and any
//! non-finite intent component — a velocity or rate off the network must never
//! reach an adapter as `NaN`.

use super::ConvertError;
use crate::intent::{
    AttitudeThrustIntent, BodyRateIntent, ControlAction, ControlIntent, GimbalRateIntent,
    PositionHoldIntent, ReferenceFrame, VelocityIntent,
};
use crate::wire;

fn frame_to_wire(frame: ReferenceFrame) -> wire::ReferenceFrame {
    match frame {
        ReferenceFrame::BodyFrd => wire::ReferenceFrame::BodyFrd,
        ReferenceFrame::LocalNed => wire::ReferenceFrame::LocalNed,
        ReferenceFrame::Gimbal => wire::ReferenceFrame::Gimbal,
    }
}

fn frame_from_wire(value: i32) -> Result<ReferenceFrame, ConvertError> {
    match wire::ReferenceFrame::try_from(value) {
        Ok(wire::ReferenceFrame::BodyFrd) => Ok(ReferenceFrame::BodyFrd),
        Ok(wire::ReferenceFrame::LocalNed) => Ok(ReferenceFrame::LocalNed),
        Ok(wire::ReferenceFrame::Gimbal) => Ok(ReferenceFrame::Gimbal),
        Ok(wire::ReferenceFrame::Unspecified) | Err(_) => Err(ConvertError::UnknownEnum {
            enum_name: "pilotage.v1.ReferenceFrame",
            value,
        }),
    }
}

pub(super) fn action_to_wire(action: ControlAction) -> wire::ControlAction {
    match action {
        ControlAction::Arm => wire::ControlAction::Arm,
        ControlAction::Disarm => wire::ControlAction::Disarm,
        ControlAction::ModeRequest => wire::ControlAction::ModeRequest,
        ControlAction::GimbalRecenter => wire::ControlAction::GimbalRecenter,
    }
}

pub(super) fn action_from_wire(value: i32) -> Result<ControlAction, ConvertError> {
    match wire::ControlAction::try_from(value) {
        Ok(wire::ControlAction::Arm) => Ok(ControlAction::Arm),
        Ok(wire::ControlAction::Disarm) => Ok(ControlAction::Disarm),
        Ok(wire::ControlAction::ModeRequest) => Ok(ControlAction::ModeRequest),
        Ok(wire::ControlAction::GimbalRecenter) => Ok(ControlAction::GimbalRecenter),
        Ok(wire::ControlAction::Unspecified) | Err(_) => Err(ConvertError::UnknownEnum {
            enum_name: "pilotage.v1.ControlAction",
            value,
        }),
    }
}

fn finite(value: f32, field: &'static str) -> Result<f32, ConvertError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(ConvertError::NonFiniteIntentValue { field })
    }
}

pub(super) fn intent_to_wire(intent: &ControlIntent) -> wire::ControlIntent {
    use wire::control_intent::Family;
    let family = match *intent {
        ControlIntent::Velocity(v) => Family::Velocity(wire::VelocityIntent {
            frame: frame_to_wire(v.frame) as i32,
            vx: v.vx,
            vy: v.vy,
            vz: v.vz,
            yaw_rate: v.yaw_rate,
        }),
        ControlIntent::PositionHold(p) => Family::PositionHold(wire::PositionHoldIntent {
            frame: frame_to_wire(p.frame) as i32,
            x: p.x,
            y: p.y,
            z: p.z,
            heading: p.heading,
        }),
        ControlIntent::AttitudeThrust(a) => Family::AttitudeThrust(wire::AttitudeThrustIntent {
            frame: frame_to_wire(a.frame) as i32,
            qw: a.qw,
            qx: a.qx,
            qy: a.qy,
            qz: a.qz,
            thrust: a.thrust,
        }),
        ControlIntent::BodyRate(b) => Family::BodyRate(wire::BodyRateIntent {
            roll_rate: b.roll_rate,
            pitch_rate: b.pitch_rate,
            yaw_rate: b.yaw_rate,
            thrust: b.thrust,
        }),
        ControlIntent::GimbalRate(g) => Family::GimbalRate(wire::GimbalRateIntent {
            pitch_rate: g.pitch_rate,
            yaw_rate: g.yaw_rate,
        }),
    };
    wire::ControlIntent {
        family: Some(family),
    }
}

pub(super) fn intent_from_wire(intent: wire::ControlIntent) -> Result<ControlIntent, ConvertError> {
    use wire::control_intent::Family;
    let family = intent.family.ok_or(ConvertError::MissingField {
        message: "pilotage.v1.ControlIntent",
        field: "family",
    })?;
    Ok(match family {
        Family::Velocity(v) => ControlIntent::Velocity(VelocityIntent {
            frame: frame_from_wire(v.frame)?,
            vx: finite(v.vx, "velocity.vx")?,
            vy: finite(v.vy, "velocity.vy")?,
            vz: finite(v.vz, "velocity.vz")?,
            yaw_rate: finite(v.yaw_rate, "velocity.yaw_rate")?,
        }),
        Family::PositionHold(p) => ControlIntent::PositionHold(PositionHoldIntent {
            frame: frame_from_wire(p.frame)?,
            x: finite(p.x, "position_hold.x")?,
            y: finite(p.y, "position_hold.y")?,
            z: finite(p.z, "position_hold.z")?,
            heading: finite(p.heading, "position_hold.heading")?,
        }),
        Family::AttitudeThrust(a) => ControlIntent::AttitudeThrust(AttitudeThrustIntent {
            frame: frame_from_wire(a.frame)?,
            qw: finite(a.qw, "attitude_thrust.qw")?,
            qx: finite(a.qx, "attitude_thrust.qx")?,
            qy: finite(a.qy, "attitude_thrust.qy")?,
            qz: finite(a.qz, "attitude_thrust.qz")?,
            thrust: finite(a.thrust, "attitude_thrust.thrust")?,
        }),
        Family::BodyRate(b) => ControlIntent::BodyRate(BodyRateIntent {
            roll_rate: finite(b.roll_rate, "body_rate.roll_rate")?,
            pitch_rate: finite(b.pitch_rate, "body_rate.pitch_rate")?,
            yaw_rate: finite(b.yaw_rate, "body_rate.yaw_rate")?,
            thrust: finite(b.thrust, "body_rate.thrust")?,
        }),
        Family::GimbalRate(g) => ControlIntent::GimbalRate(GimbalRateIntent {
            pitch_rate: finite(g.pitch_rate, "gimbal_rate.pitch_rate")?,
            yaw_rate: finite(g.yaw_rate, "gimbal_rate.yaw_rate")?,
        }),
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        ControlAction, ControlIntent, ConvertError, GimbalRateIntent, ReferenceFrame,
        VelocityIntent, action_from_wire, action_to_wire, intent_from_wire, intent_to_wire, wire,
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
            ControlAction::ModeRequest,
            ControlAction::GimbalRecenter,
        ] {
            let wire_value = action_to_wire(action) as i32;
            assert_eq!(action_from_wire(wire_value).expect("round-trips"), action);
        }
        assert!(matches!(
            action_from_wire(wire::ControlAction::Unspecified as i32),
            Err(ConvertError::UnknownEnum {
                enum_name: "pilotage.v1.ControlAction",
                ..
            })
        ));
    }
}
