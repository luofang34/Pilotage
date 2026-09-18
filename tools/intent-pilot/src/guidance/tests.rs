//! Guidance behaviour: single demands, then closed loops on two plants.

#![allow(clippy::panic)]

use pilotage_client_session::MotionDemand;

use super::{STOP, distance_m, ground_speed_mps, toward, wrap_pi};
use crate::scenario::Waypoint;
use crate::vehicle_state::VehicleState;

const EAST: Waypoint = Waypoint {
    north_m: 0.0,
    east_m: 15.0,
};
const RADIUS_M: f64 = 1.5;
const DT_S: f64 = 0.05;

fn at_origin(yaw_rad: f64) -> VehicleState {
    VehicleState {
        north_m: 0.0,
        east_m: 0.0,
        vel_north_mps: 0.0,
        vel_east_mps: 0.0,
        height_m: Some(5.0),
        climb_mps: 0.0,
        yaw_rad,
        armed: Some(true),
    }
}

#[test]
fn a_waypoint_abeam_demands_a_turn_and_no_forward_motion() {
    let demand = toward(&at_origin(0.0), EAST, 2.5, 0.0);
    assert!(demand.yaw > 0.9, "right yaw expected, got {}", demand.yaw);
    assert!(demand.pitch.abs() < 0.01, "got {}", demand.pitch);
    assert!(
        demand.roll.abs() < f32::EPSILON,
        "no sideways demand far out"
    );
}

#[test]
fn a_vehicle_that_faces_a_far_waypoint_flies_forward_with_no_yaw() {
    let demand = toward(&at_origin(std::f64::consts::FRAC_PI_2), EAST, 2.5, 0.0);
    assert!(demand.yaw.abs() < 1e-6);
    assert!(demand.pitch > 0.99);
}

#[test]
fn a_fast_vehicle_close_to_the_waypoint_gets_a_reverse_demand() {
    let mut state = at_origin(std::f64::consts::FRAC_PI_2);
    state.east_m = 13.0;
    state.vel_east_mps = 5.0;
    let demand = toward(&state, EAST, 1.0, 0.0);
    assert!(demand.pitch < -0.9, "must brake, got {}", demand.pitch);
    assert!(
        demand.yaw.abs() < f32::EPSILON,
        "no steering inside station range"
    );
}

#[test]
fn a_waypoint_behind_the_vehicle_turns_the_short_way() {
    let west = Waypoint {
        north_m: 0.0,
        east_m: -15.0,
    };
    // The heading is a little west of south, so the short turn is right.
    assert!(toward(&at_origin(-3.0), west, 2.5, 0.0).yaw > 0.0);
}

#[test]
fn wrapping_keeps_the_half_open_interval() {
    assert!((wrap_pi(3.0 * std::f64::consts::PI) - std::f64::consts::PI).abs() < 1e-9);
    assert!((wrap_pi(-0.5) + 0.5).abs() < 1e-9);
}

/// Moves `state` one step under `demand`. `tracks_velocity` selects the
/// plant: a multirotor that tracks a velocity demand on both body axes, or
/// a boat that reads the demand as throttle, cannot move sideways, and
/// stops only on reverse throttle and drag.
fn advance(state: &mut VehicleState, demand: MotionDemand, tracks_velocity: bool) {
    let (sin, cos) = state.yaw_rad.sin_cos();
    let (pitch, roll) = (f64::from(demand.pitch), f64::from(demand.roll));
    if tracks_velocity {
        const LIMIT_MPS: f64 = 2.5;
        const LAG_S: f64 = 0.8;
        let want_north = LIMIT_MPS * (cos * pitch - sin * roll);
        let want_east = LIMIT_MPS * (sin * pitch + cos * roll);
        state.vel_north_mps += (want_north - state.vel_north_mps) * DT_S / LAG_S;
        state.vel_east_mps += (want_east - state.vel_east_mps) * DT_S / LAG_S;
    } else {
        const THRUST_MPS2: f64 = 3.0;
        const DRAG_PER_S: f64 = 0.5;
        let surge = cos * state.vel_north_mps + sin * state.vel_east_mps;
        let surge = surge + (THRUST_MPS2 * pitch - DRAG_PER_S * surge) * DT_S;
        state.vel_north_mps = surge * cos;
        state.vel_east_mps = surge * sin;
    }
    state.yaw_rad += f64::from(demand.yaw) * DT_S;
    state.north_m += state.vel_north_mps * DT_S;
    state.east_m += state.vel_east_mps * DT_S;
}

/// Flies to `EAST` from the origin, heading north, for 60 s. Returns the
/// greatest distance from the waypoint after the first entry into the
/// arrival radius, and the final state.
fn fly(tracks_velocity: bool, advertised_limit_mps: f64) -> (Option<f64>, VehicleState) {
    let mut state = at_origin(0.0);
    let mut worst_after_entry: Option<f64> = None;
    for _ in 0..1200 {
        let demand = toward(&state, EAST, advertised_limit_mps, 0.0);
        advance(&mut state, demand, tracks_velocity);
        let range = distance_m(&state, EAST);
        if range <= RADIUS_M || worst_after_entry.is_some() {
            worst_after_entry = Some(worst_after_entry.map_or(range, |worst| worst.max(range)));
        }
    }
    (worst_after_entry, state)
}

fn assert_captured(plant: &str, worst_after_entry: Option<f64>, last: &VehicleState) {
    let Some(worst) = worst_after_entry else {
        panic!(
            "{plant}: never came inside {RADIUS_M} m; ended {:.2} m away",
            distance_m(last, EAST)
        );
    };
    assert!(
        worst <= RADIUS_M,
        "{plant}: left the radius after entry, to {worst:.2} m"
    );
    assert!(
        distance_m(last, EAST) <= 0.5,
        "{plant}: ended {:.2} m away",
        distance_m(last, EAST)
    );
    assert!(ground_speed_mps(last) <= 0.1, "{plant}: still moving");
}

#[test]
fn a_velocity_tracking_multirotor_is_captured_at_the_waypoint() {
    let (worst, last) = fly(true, 2.5);
    assert_captured("multirotor", worst, &last);
}

#[test]
fn a_throttle_driven_boat_that_outruns_its_advertised_speed_is_captured_too() {
    // The plant reaches 6 m/s, and the advertisement says 1 m/s. The
    // reference adapter has the same mismatch.
    let (worst, last) = fly(false, 1.0);
    assert_captured("boat", worst, &last);
}

#[test]
fn a_demand_cut_at_the_radius_lets_the_boat_run_through_the_waypoint() {
    // The defect this law replaces: full demand to the arrival radius and a
    // motionless demand inside it. The run-through is what makes the
    // capture tests above mean something.
    let mut state = at_origin(std::f64::consts::FRAC_PI_2);
    let mut overshoot: f64 = 0.0;
    for _ in 0..1200 {
        let outside = distance_m(&state, EAST) > RADIUS_M && state.east_m < EAST.east_m;
        let demand = if outside {
            MotionDemand { pitch: 1.0, ..STOP }
        } else {
            STOP
        };
        advance(&mut state, demand, false);
        overshoot = overshoot.max(state.east_m - EAST.east_m);
    }
    assert!(
        overshoot > 5.0,
        "the naive law overshoots by {overshoot:.1} m"
    );
}
