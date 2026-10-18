#![allow(clippy::expect_used, clippy::panic)]

use pilotage_protocol::wire;

use super::{attitude_thrust_intent, body_rate_intent, integrate_heading};
use crate::MotionDemand;

fn capability(
    family: wire::IntentFamily,
    frames: &[wire::ReferenceFrame],
) -> wire::IntentCapability {
    wire::IntentCapability {
        family: family as i32,
        frames: frames.iter().map(|frame| *frame as i32).collect(),
        max_angular: 0.5,
        max_yaw_rate: 1.5,
        ..Default::default()
    }
}

fn demand(roll: f32, pitch: f32, throttle: f32, yaw: f32) -> MotionDemand {
    MotionDemand {
        roll,
        pitch,
        throttle,
        yaw,
    }
}

fn attitude(intent: Option<wire::ControlIntent>) -> wire::AttitudeThrustIntent {
    match intent.and_then(|intent| intent.family) {
        Some(wire::control_intent::Family::AttitudeThrust(attitude)) => attitude,
        other => panic!("expected an attitude intent, got {other:?}"),
    }
}

fn rates(intent: Option<wire::ControlIntent>) -> wire::BodyRateIntent {
    match intent.and_then(|intent| intent.family) {
        Some(wire::control_intent::Family::BodyRate(rates)) => rates,
        other => panic!("expected a body-rate intent, got {other:?}"),
    }
}

/// Pitch angle (rad, positive nose up) of an FRD → NED quaternion.
fn pitch_of(q: &wire::AttitudeThrustIntent) -> f32 {
    (2.0 * (q.qw * q.qy - q.qz * q.qx)).clamp(-1.0, 1.0).asin()
}

fn roll_of(q: &wire::AttitudeThrustIntent) -> f32 {
    (2.0 * (q.qw * q.qx + q.qy * q.qz)).atan2(1.0 - 2.0 * (q.qx * q.qx + q.qy * q.qy))
}

#[test]
fn an_unadvertised_or_wrong_family_builds_nothing() {
    let neutral = demand(0.0, 0.0, 0.0, 0.0);
    let velocity = capability(
        wire::IntentFamily::Velocity,
        &[wire::ReferenceFrame::LocalNed],
    );
    assert!(attitude_thrust_intent(neutral, 0.0, None).is_none());
    assert!(attitude_thrust_intent(neutral, 0.0, Some(&velocity)).is_none());
    assert!(body_rate_intent(neutral, None).is_none());
    assert!(body_rate_intent(neutral, Some(&velocity)).is_none());
}

#[test]
fn angle_flight_needs_the_local_ned_frame() {
    let body_only = capability(
        wire::IntentFamily::AttitudeThrust,
        &[wire::ReferenceFrame::BodyFrd],
    );
    assert!(attitude_thrust_intent(demand(0.0, 0.0, 0.0, 0.0), 0.0, Some(&body_only)).is_none());
}

#[test]
fn a_forward_demand_tilts_the_nose_down_to_the_advertised_bound() {
    let angle = capability(
        wire::IntentFamily::AttitudeThrust,
        &[wire::ReferenceFrame::LocalNed],
    );
    let forward = attitude(attitude_thrust_intent(
        demand(0.0, 1.0, 0.0, 0.0),
        0.0,
        Some(&angle),
    ));
    assert!(
        (pitch_of(&forward) + 0.5).abs() < 1e-5,
        "nose down by max_angular"
    );
    assert!(
        (forward.thrust - 0.5).abs() < 1e-6,
        "a centered stick is half thrust"
    );
    let right = attitude(attitude_thrust_intent(
        demand(1.0, 0.0, -1.0, 0.0),
        0.0,
        Some(&angle),
    ));
    assert!(
        (roll_of(&right) - 0.5).abs() < 1e-5,
        "right roll by max_angular"
    );
    assert_eq!(right.thrust, 0.0, "full down is zero thrust");
}

#[test]
fn the_heading_slews_at_the_advertised_yaw_rate_and_wraps() {
    let angle = capability(
        wire::IntentFamily::AttitudeThrust,
        &[wire::ReferenceFrame::LocalNed],
    );
    let heading = integrate_heading(0.0, 1.0, Some(&angle), 0.1);
    assert!((heading - 0.15).abs() < 1e-6);
    let wrapped = integrate_heading(3.1, 1.0, Some(&angle), 0.1);
    assert!(
        wrapped < 0.0 && wrapped > -std::f32::consts::PI,
        "{wrapped}"
    );
    assert_eq!(
        integrate_heading(0.2, 1.0, None, 0.1),
        0.2,
        "no advertisement, no slew"
    );
}

#[test]
fn acro_demands_scale_to_body_rates_with_nose_down_for_forward() {
    let acro = capability(wire::IntentFamily::BodyRate, &[]);
    let full = rates(body_rate_intent(demand(1.0, 1.0, 1.0, -1.0), Some(&acro)));
    assert_eq!(full.roll_rate, 0.5);
    assert_eq!(full.pitch_rate, -0.5, "forward stick pitches the nose down");
    assert_eq!(full.yaw_rate, -1.5, "yaw uses max_yaw_rate");
    assert_eq!(full.thrust, 1.0);
    let no_yaw_bound = wire::IntentCapability {
        max_yaw_rate: 0.0,
        ..acro
    };
    let yaw = rates(body_rate_intent(
        demand(0.0, 0.0, 0.0, 1.0),
        Some(&no_yaw_bound),
    ));
    assert_eq!(yaw.yaw_rate, 0.5, "yaw falls back to max_angular");
}
