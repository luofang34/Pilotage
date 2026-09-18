//! Guidance behaviour: single demands, then closed loops on two plants.

#![allow(clippy::panic)]

use super::{Demand, Speeds, along_heading, distance_m, ground_speed_mps, toward, wrap_pi};
use crate::directive::TurnDirection;
use crate::scenario::Fix;
use crate::state::VehicleState;

const EAST: Fix = Fix {
    north_m: 0.0,
    east_m: 15.0,
};
const RADIUS_M: f64 = 1.5;
const DT_S: f64 = 0.05;
const QUAD: Speeds = Speeds {
    max_linear_mps: 3.0,
    cruise_mps: 2.4,
};

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
fn a_fix_abeam_demands_a_turn_and_no_forward_motion() {
    let demand = toward(&at_origin(0.0), EAST, QUAD, 0.0);
    assert!(demand.yaw > 0.9, "right yaw expected, got {}", demand.yaw);
    assert!(demand.pitch.abs() < 0.01, "got {}", demand.pitch);
}

#[test]
fn a_fast_vehicle_close_to_the_fix_gets_a_reverse_demand() {
    let mut state = at_origin(std::f64::consts::FRAC_PI_2);
    state.east_m = 13.0;
    state.vel_east_mps = 5.0;
    let boat = Speeds {
        max_linear_mps: 1.0,
        cruise_mps: 0.8,
    };
    let demand = toward(&state, EAST, boat, 0.0);
    assert!(demand.pitch < -0.9, "must brake, got {}", demand.pitch);
    assert!(
        demand.yaw.abs() < f32::EPSILON,
        "no steering inside station range"
    );
}

#[test]
fn a_commanded_left_turn_goes_left_when_right_is_shorter() {
    // Heading 090 from north: the short way is right.
    let east = std::f64::consts::FRAC_PI_2;
    let shortest = along_heading(&at_origin(0.0), east, TurnDirection::Shortest, QUAD, 0.0);
    let left = along_heading(&at_origin(0.0), east, TurnDirection::Left, QUAD, 0.0);
    assert!(shortest.yaw > 0.9);
    assert!(left.yaw < -0.9, "got {}", left.yaw);
}

#[test]
fn a_commanded_side_is_dropped_when_the_nose_is_near_the_heading() {
    let nearly = std::f64::consts::FRAC_PI_2 - 0.1;
    let demand = along_heading(
        &at_origin(nearly),
        std::f64::consts::FRAC_PI_2,
        TurnDirection::Left,
        QUAD,
        0.0,
    );
    assert!(
        demand.yaw > 0.0,
        "the last tenth of a radian goes the short way"
    );
}

#[test]
fn wrapping_keeps_the_half_open_interval() {
    assert!((wrap_pi(3.0 * std::f64::consts::PI) - std::f64::consts::PI).abs() < 1e-9);
    assert!((wrap_pi(-0.5) + 0.5).abs() < 1e-9);
}

/// Moves `state` one step under `demand`. `tracks_velocity` selects the plant:
/// a multirotor that tracks a velocity demand on both body axes, or a boat
/// that reads the demand as throttle, cannot move sideways, and stops only on
/// reverse throttle and drag.
fn advance(state: &mut VehicleState, demand: Demand, tracks_velocity: bool) {
    let (sin, cos) = state.yaw_rad.sin_cos();
    let (pitch, roll) = (f64::from(demand.pitch), f64::from(demand.roll));
    if tracks_velocity {
        const LIMIT_MPS: f64 = 3.0;
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
    state.yaw_rad += 0.9 * f64::from(demand.yaw) * DT_S;
    state.north_m += state.vel_north_mps * DT_S;
    state.east_m += state.vel_east_mps * DT_S;
}

fn assert_captured(plant: &str, tracks_velocity: bool, speeds: Speeds) {
    let mut state = at_origin(0.0);
    let mut worst_after_entry: Option<f64> = None;
    for _ in 0..1200 {
        let demand = toward(&state, EAST, speeds, 0.0);
        advance(&mut state, demand, tracks_velocity);
        let range = distance_m(&state, EAST);
        if range <= RADIUS_M || worst_after_entry.is_some() {
            worst_after_entry = Some(worst_after_entry.map_or(range, |worst| worst.max(range)));
        }
    }
    let Some(worst) = worst_after_entry else {
        panic!("{plant}: never came inside {RADIUS_M} m");
    };
    assert!(
        worst <= RADIUS_M,
        "{plant}: left the radius after entry, to {worst:.2} m"
    );
    assert!(
        distance_m(&state, EAST) <= 0.5,
        "{plant}: ended away from the fix"
    );
    assert!(ground_speed_mps(&state) <= 0.1, "{plant}: still moving");
}

#[test]
fn a_velocity_tracking_multirotor_is_captured_at_the_fix() {
    assert_captured("multirotor", true, QUAD);
}

#[test]
fn a_throttle_driven_boat_that_outruns_its_advertised_speed_is_captured_too() {
    // The plant reaches 6 m/s, and the advertisement says 1 m/s. The host
    // reference adapter has the same mismatch.
    let boat = Speeds {
        max_linear_mps: 1.0,
        cruise_mps: 0.8,
    };
    assert_captured("boat", false, boat);
}

#[test]
fn a_demand_cut_at_the_radius_lets_the_boat_run_through_the_fix() {
    // The defect that the speed schedule prevents: full demand to the arrival
    // radius and no demand inside it. The run-through is what makes the
    // capture tests mean something.
    let mut state = at_origin(std::f64::consts::FRAC_PI_2);
    let mut overshoot: f64 = 0.0;
    for _ in 0..1200 {
        let outside = distance_m(&state, EAST) > RADIUS_M && state.east_m < EAST.east_m;
        let demand = Demand {
            pitch: if outside { 1.0 } else { 0.0 },
            ..Demand::default()
        };
        advance(&mut state, demand, false);
        overshoot = overshoot.max(state.east_m - EAST.east_m);
    }
    assert!(
        overshoot > 5.0,
        "the naive law overshoots by {overshoot:.1} m"
    );
}

#[test]
fn a_left_turn_to_west_the_long_way_ends_on_a_westerly_track() {
    // From heading 090, west by the left is 180 degrees either way, so start
    // at 100: the short way to 270 is right, and the command says left.
    let mut state = at_origin(100.0_f64.to_radians());
    let west = 270.0_f64.to_radians();
    let mut turned_left = false;
    for _ in 0..600 {
        let demand = along_heading(&state, west, TurnDirection::Left, QUAD, 0.0);
        turned_left |= demand.yaw < -0.5;
        assert!(
            demand.yaw < 0.2,
            "never a hard right turn, got {}",
            demand.yaw
        );
        advance(&mut state, demand, true);
    }
    assert!(turned_left);
    assert!(
        wrap_pi(state.yaw_rad - west).abs() < 0.05,
        "ends on the heading"
    );
    assert!(state.vel_east_mps < -2.0, "flies west at cruise speed");
    assert!(
        state.vel_north_mps.abs() < 0.1,
        "with no drift across the track"
    );
}
