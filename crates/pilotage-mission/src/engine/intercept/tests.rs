use navigate_contract::NedVelocity;

use super::*;

fn course_of(setpoint: GuidanceSetpoint) -> f64 {
    let GuidanceSetpoint::Velocity { velocity } = setpoint else {
        panic!("a velocity stays a velocity");
    };
    velocity.east_mps.atan2(velocity.north_mps)
}

fn toward(course_deg: f64) -> GuidanceSetpoint {
    let course = course_deg.to_radians();
    GuidanceSetpoint::Velocity {
        velocity: NedVelocity::new(60.0 * course.cos(), 60.0 * course.sin(), 1.0),
    }
}

#[test]
fn a_course_square_to_the_track_is_held_at_45_degrees_on_its_side() {
    let track = 0.0;
    assert!((course_of(within_intercept(toward(80.0), track)) - FRAC_PI_4).abs() < 1e-12);
    assert!((course_of(within_intercept(toward(-80.0), track)) + FRAC_PI_4).abs() < 1e-12);
}

#[test]
fn a_course_within_the_intercept_and_the_speed_are_kept() {
    assert_eq!(within_intercept(toward(30.0), 0.0), toward(30.0));
    let GuidanceSetpoint::Velocity { velocity } = within_intercept(toward(100.0), 1.0) else {
        panic!("a velocity stays a velocity");
    };
    assert!((velocity.north_mps.hypot(velocity.east_mps) - 60.0).abs() < 1e-9);
    assert_eq!(velocity.down_mps, 1.0);
}
