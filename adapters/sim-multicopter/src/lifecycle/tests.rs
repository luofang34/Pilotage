use super::{FlightPhase, TAKEOFF_HEIGHT_M};
use crate::config::Limits;
use crate::kinematics::{BodyVelocity, Kinematics};

const DT: f64 = 0.01;

fn run(phase: &mut FlightPhase, body: &mut Kinematics, command: BodyVelocity, seconds: f64) {
    let limits = Limits::default();
    for _ in 0..(seconds / DT).round() as u32 {
        phase.step(body, command, &limits, DT);
    }
}

#[test]
fn takeoff_climbs_to_the_hover_height_and_hovers() {
    let mut phase = FlightPhase::Disarmed;
    let mut body = Kinematics::at_rest(0.0);
    assert_eq!(phase.takeoff(), Ok(()));
    assert!(phase.armed());
    run(&mut phase, &mut body, BodyVelocity::default(), 10.0);
    assert_eq!(phase, FlightPhase::Airborne);
    assert!(
        (body.height_m() - TAKEOFF_HEIGHT_M).abs() < 0.15,
        "{}",
        body.height_m()
    );
    assert!(body.vel_ned_mps[2].abs() < 0.01);
}

#[test]
fn an_armed_vehicle_lifts_off_on_a_climb_command_only() {
    let mut phase = FlightPhase::Disarmed;
    let mut body = Kinematics::at_rest(0.0);
    assert_eq!(phase.arm(), Ok(()));
    let forward = BodyVelocity {
        forward: 5.0,
        ..BodyVelocity::default()
    };
    run(&mut phase, &mut body, forward, 2.0);
    assert_eq!(phase, FlightPhase::ArmedOnGround);
    assert_eq!(body.pos_ned_m, [0.0; 3], "no sliding on the ground");
    let climb = BodyVelocity {
        down: -2.0,
        ..BodyVelocity::default()
    };
    run(&mut phase, &mut body, climb, 5.0);
    assert_eq!(phase, FlightPhase::Airborne);
    assert!(body.height_m() > 7.0, "{}", body.height_m());
}

#[test]
fn landing_descends_and_stops_the_motors() {
    let mut phase = FlightPhase::Airborne;
    let mut body = Kinematics::at_rest(0.0);
    body.pos_ned_m[2] = -10.0;
    assert_eq!(phase.land(), Ok(()));
    assert!(phase.admits_velocity().is_err());
    run(&mut phase, &mut body, BodyVelocity::default(), 20.0);
    assert_eq!(phase, FlightPhase::Disarmed);
    assert_eq!(body.height_m(), 0.0);
}

#[test]
fn a_velocity_descent_to_the_ground_keeps_the_motors_armed() {
    let mut phase = FlightPhase::Airborne;
    let mut body = Kinematics::at_rest(0.0);
    body.pos_ned_m[2] = -2.0;
    let descend = BodyVelocity {
        down: 1.0,
        ..BodyVelocity::default()
    };
    run(&mut phase, &mut body, descend, 6.0);
    assert_eq!(phase, FlightPhase::ArmedOnGround);
    assert_eq!(phase.disarm(), Ok(()));
}

#[test]
fn refusals_name_the_reason() {
    let mut phase = FlightPhase::Disarmed;
    assert!(phase.admits_velocity().is_err());
    assert!(phase.land().is_err());
    let mut flying = FlightPhase::Airborne;
    assert!(flying.disarm().is_err());
    assert!(flying.takeoff().is_err());
    assert_eq!(flying, FlightPhase::Airborne);
}
