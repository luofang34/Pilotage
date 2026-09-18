//! Sequencer behaviour, one decision per test.

use super::{Destination, Discrete, FlightLimits, Phase, Sequencer};
use crate::scenario::{Arrival, Waypoint};
use crate::vehicle_state::VehicleState;

const LIMITS: FlightLimits = FlightLimits {
    cruise_height_m: 5.0,
    arrival_radius_m: 1.5,
    max_linear_mps: 2.5,
    disarm_offered: true,
};

fn state(north_m: f64, east_m: f64, height_m: f64, armed: bool) -> VehicleState {
    VehicleState {
        north_m,
        east_m,
        vel_north_mps: 0.0,
        vel_east_mps: 0.0,
        height_m: Some(height_m),
        climb_mps: 0.0,
        yaw_rad: 0.0,
        armed: Some(armed),
    }
}

fn destination(name: &str, north_m: f64, east_m: f64, on_arrival: Arrival) -> Destination {
    Destination {
        name: name.to_owned(),
        waypoint: Waypoint { north_m, east_m },
        on_arrival,
    }
}

fn alpha(on_arrival: Arrival) -> Destination {
    destination("ALPHA", 15.0, 0.0, on_arrival)
}

#[test]
fn nothing_happens_without_an_intent() {
    let mut sequencer = Sequencer::new(LIMITS);
    let step = sequencer.step(&state(0.0, 0.0, 0.0, false), 0.0);
    assert_eq!(step.phase, Phase::AwaitIntent);
    assert_eq!(step.action, None);
}

#[test]
fn an_intent_on_the_ground_arms_then_climbs_then_flies() {
    let mut sequencer = Sequencer::new(LIMITS);
    sequencer.set_destination(alpha(Arrival::Land));

    assert_eq!(
        sequencer.step(&state(0.0, 0.0, 0.0, false), 0.0).phase,
        Phase::Arming
    );
    let arm = sequencer.step(&state(0.0, 0.0, 0.0, false), 0.05);
    assert_eq!(arm.action, Some(Discrete::Arm));

    assert_eq!(
        sequencer.step(&state(0.0, 0.0, 0.0, true), 0.1).phase,
        Phase::Climb
    );
    let climbing = sequencer.step(&state(0.0, 0.0, 1.0, true), 0.15);
    assert!(climbing.demand.throttle > 0.5);
    assert!(climbing.demand.pitch.abs() < f32::EPSILON);

    assert_eq!(
        sequencer.step(&state(0.0, 0.0, 4.8, true), 5.0).phase,
        Phase::Enroute
    );
    let flying = sequencer.step(&state(0.0, 0.0, 5.0, true), 5.05);
    assert!(flying.demand.pitch > 0.9, "ALPHA is dead ahead");
}

#[test]
fn an_unanswered_arm_request_repeats_only_after_the_retry_interval() {
    let mut sequencer = Sequencer::new(LIMITS);
    sequencer.set_destination(alpha(Arrival::Land));
    let ground = state(0.0, 0.0, 0.0, false);
    sequencer.step(&ground, 0.0);
    assert_eq!(sequencer.step(&ground, 0.05).action, Some(Discrete::Arm));
    assert_eq!(sequencer.step(&ground, 1.0).action, None);
    assert_eq!(sequencer.step(&ground, 2.1).action, Some(Discrete::Arm));
}

fn airborne_enroute(on_arrival: Arrival) -> Sequencer {
    let mut sequencer = Sequencer::new(LIMITS);
    sequencer.set_destination(alpha(on_arrival));
    sequencer.step(&state(0.0, 0.0, 0.0, false), 0.0);
    sequencer.step(&state(0.0, 0.0, 0.0, true), 0.1);
    sequencer.step(&state(0.0, 0.0, 5.0, true), 1.0);
    assert_eq!(sequencer.phase(), Phase::Enroute);
    sequencer
}

#[test]
fn arrival_with_land_descends_then_disarms_on_the_ground() {
    let mut sequencer = airborne_enroute(Arrival::Land);
    assert_eq!(
        sequencer.step(&state(14.5, 0.0, 5.0, true), 10.0).phase,
        Phase::Descending
    );
    let descending = sequencer.step(&state(14.5, 0.0, 3.0, true), 11.0);
    assert!(descending.demand.throttle < 0.0);
    assert_eq!(descending.action, None, "no disarm in the air");

    assert_eq!(
        sequencer.step(&state(14.5, 0.0, 0.2, true), 15.0).phase,
        Phase::Disarming
    );
    let disarm = sequencer.step(&state(14.5, 0.0, 0.2, true), 15.05);
    assert_eq!(disarm.action, Some(Discrete::Disarm));
    assert_eq!(
        sequencer.step(&state(14.5, 0.0, 0.2, false), 15.5).phase,
        Phase::Landed
    );
}

#[test]
fn a_descent_that_stops_above_zero_height_still_counts_as_touchdown() {
    let mut sequencer = airborne_enroute(Arrival::Land);
    sequencer.step(&state(14.5, 0.0, 5.0, true), 10.0);
    // The estimate settles at 0.9 m. It never reads the 0.3 m ground band.
    let settled = state(14.5, 0.0, 0.9, true);
    assert_eq!(sequencer.step(&settled, 12.0).phase, Phase::Descending);
    assert_eq!(sequencer.step(&settled, 13.0).phase, Phase::Descending);
    assert_eq!(sequencer.step(&settled, 14.1).phase, Phase::Disarming);
}

#[test]
fn arrival_with_hold_stays_in_the_air_and_never_disarms() {
    let mut sequencer = airborne_enroute(Arrival::Hold);
    assert_eq!(
        sequencer.step(&state(14.5, 0.0, 5.0, true), 10.0).phase,
        Phase::Holding
    );
    for tick in 0..100 {
        let step = sequencer.step(&state(14.5, 0.0, 5.0, true), 10.0 + f64::from(tick) * 0.05);
        assert_eq!(step.phase, Phase::Holding);
        assert_eq!(step.action, None);
        assert!(
            step.demand.yaw.abs() < f32::EPSILON,
            "no steering on station"
        );
        assert!(step.demand.pitch > 0.0, "closes the last half metre");
        assert!(step.demand.pitch < 0.5, "and does it gently");
    }
}

#[test]
fn a_new_destination_in_flight_turns_the_vehicle_at_once() {
    let mut sequencer = airborne_enroute(Arrival::Land);
    let north = sequencer.step(&state(5.0, 0.0, 5.0, true), 3.0);
    assert!(north.demand.yaw.abs() < 1e-3, "ALPHA is dead ahead");

    sequencer.set_destination(destination("BRAVO", 5.0, 15.0, Arrival::Land));
    let east = sequencer.step(&state(5.0, 0.0, 5.0, true), 3.05);
    assert_eq!(east.phase, Phase::Enroute);
    assert!(east.demand.yaw > 0.9, "BRAVO is abeam to the right");
}

#[test]
fn a_new_destination_abandons_a_landing_in_progress() {
    let mut sequencer = airborne_enroute(Arrival::Land);
    sequencer.step(&state(14.5, 0.0, 5.0, true), 10.0);
    sequencer.step(&state(14.5, 0.0, 3.0, true), 11.0);
    assert_eq!(sequencer.phase(), Phase::Descending);

    sequencer.set_destination(destination("HOME", 0.0, 0.0, Arrival::Land));
    let step = sequencer.step(&state(14.5, 0.0, 3.0, true), 11.05);
    assert_eq!(step.phase, Phase::Enroute);
    assert!(step.demand.throttle > 0.0, "climbs back to cruise height");
}

#[test]
fn a_destination_that_arrives_during_disarm_is_flown_after_it() {
    let mut sequencer = airborne_enroute(Arrival::Land);
    sequencer.step(&state(14.5, 0.0, 5.0, true), 10.0);
    sequencer.step(&state(14.5, 0.0, 0.2, true), 15.0);
    assert_eq!(sequencer.phase(), Phase::Disarming);

    sequencer.set_destination(destination("HOME", 0.0, 0.0, Arrival::Land));
    assert_eq!(
        sequencer.step(&state(14.5, 0.0, 0.2, false), 15.5).phase,
        Phase::Landed
    );
    assert_eq!(
        sequencer.step(&state(14.5, 0.0, 0.2, false), 15.55).phase,
        Phase::Arming
    );
}

#[test]
fn a_landed_vehicle_with_a_served_intent_stays_on_the_ground() {
    let mut sequencer = airborne_enroute(Arrival::Land);
    sequencer.step(&state(14.5, 0.0, 5.0, true), 10.0);
    sequencer.step(&state(14.5, 0.0, 0.2, true), 15.0);
    sequencer.step(&state(14.5, 0.0, 0.2, false), 15.5);
    for tick in 0..40 {
        let step = sequencer.step(&state(14.5, 0.0, 0.2, false), 16.0 + f64::from(tick) * 0.05);
        assert_eq!(step.phase, Phase::Landed);
        assert_eq!(step.action, None);
    }
}

#[test]
fn a_planar_vehicle_skips_the_climb_and_the_descent() {
    let planar = |north_m: f64, armed: Option<bool>| VehicleState {
        north_m,
        east_m: 0.0,
        vel_north_mps: 0.0,
        vel_east_mps: 0.0,
        height_m: None,
        climb_mps: 0.0,
        yaw_rad: 0.0,
        armed,
    };
    let mut sequencer = Sequencer::new(LIMITS);
    sequencer.set_destination(alpha(Arrival::Land));
    sequencer.step(&planar(0.0, None), 0.0);
    assert_eq!(
        sequencer.step(&planar(0.0, None), 0.05).action,
        Some(Discrete::Arm)
    );
    // No armed state on the wire: the accepted result stands in for it.
    sequencer.on_action_result(Discrete::Arm, true);
    assert_eq!(
        sequencer.step(&planar(0.0, None), 0.1).phase,
        Phase::Enroute
    );
    assert_eq!(
        sequencer.step(&planar(14.5, None), 9.0).phase,
        Phase::Descending
    );
    assert_eq!(
        sequencer.step(&planar(14.5, None), 9.05).phase,
        Phase::Disarming
    );
    sequencer.on_action_result(Discrete::Disarm, true);
    assert_eq!(
        sequencer.step(&planar(14.5, None), 9.2).phase,
        Phase::Landed
    );
}

#[test]
fn enroute_time_restarts_on_each_entry() {
    let mut sequencer = airborne_enroute(Arrival::Hold);
    sequencer.step(&state(2.0, 0.0, 5.0, true), 1.05);
    let first = sequencer.enroute_seconds(4.0);
    assert!(first.is_some_and(|seconds| seconds > 2.9 && seconds < 3.1));
    sequencer.step(&state(14.5, 0.0, 5.0, true), 10.0);
    assert_eq!(
        sequencer.enroute_seconds(10.5),
        None,
        "holding is not en route"
    );
}

#[test]
fn a_vehicle_that_crosses_the_radius_at_speed_has_not_arrived() {
    let mut sequencer = airborne_enroute(Arrival::Land);
    let mut fast = state(14.5, 0.0, 5.0, true);
    fast.vel_north_mps = 2.0;
    let step = sequencer.step(&fast, 8.0);
    assert_eq!(
        step.phase,
        Phase::Enroute,
        "inside the radius, and too fast"
    );
    assert!(step.demand.pitch < 0.0, "brakes against the closing speed");
    assert!(step.demand.throttle > -0.1, "no descent while it moves");

    assert_eq!(
        sequencer.step(&state(14.8, 0.0, 5.0, true), 9.0).phase,
        Phase::Descending
    );
}

#[test]
fn a_vehicle_that_offers_no_disarm_is_landed_at_touchdown() {
    let mut sequencer = Sequencer::new(FlightLimits {
        disarm_offered: false,
        ..LIMITS
    });
    sequencer.set_destination(alpha(Arrival::Land));
    sequencer.step(&state(0.0, 0.0, 0.0, false), 0.0);
    sequencer.step(&state(0.0, 0.0, 0.0, true), 0.1);
    sequencer.step(&state(0.0, 0.0, 5.0, true), 1.0);
    sequencer.step(&state(14.8, 0.0, 5.0, true), 9.0);
    let down = sequencer.step(&state(14.8, 0.0, 0.2, true), 14.0);
    assert_eq!(down.phase, Phase::Landed);
    for tick in 0..40 {
        let step = sequencer.step(&state(14.8, 0.0, 0.2, true), 14.1 + f64::from(tick) * 0.05);
        assert_eq!(step.action, None, "an unoffered disarm is never requested");
    }
}

#[test]
fn a_new_destination_restarts_the_enroute_time() {
    let mut sequencer = airborne_enroute(Arrival::Hold);
    sequencer.step(&state(2.0, 0.0, 5.0, true), 1.05);
    assert!(sequencer.enroute_seconds(4.0).is_some_and(|s| s > 2.9));

    sequencer.set_destination(destination("BRAVO", 0.0, 15.0, Arrival::Hold));
    sequencer.step(&state(6.0, 0.0, 5.0, true), 4.05);
    let since_retarget = sequencer.enroute_seconds(5.0);
    assert!(
        since_retarget.is_some_and(|s| s < 1.0),
        "the time counts from the retarget, got {since_retarget:?}"
    );
}
