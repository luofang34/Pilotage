//! The selected targets against the engine in flight: what each target
//! does to the intents and the modes, and what the engine refuses.

mod abeam;
mod rig;

use core::f64::consts::{FRAC_PI_2, PI, TAU};

use navigate_contract::MonotonicNanos;
use pilotage_protocol::{ControlIntent, HeadingAltitudeIntent, VehicleClass};

use super::{SelectError, SelectedTargets};
use crate::{LateralMode, MissionEngine, MissionEvent, MissionOutput, MissionState, VerticalMode};
use rig::{ALTITUDE_M, CEILING_MPS, CRUISE_MPS, Feed, Rig};

fn heading(heading_true_rad: f64) -> SelectedTargets {
    SelectedTargets {
        heading_true_rad: Some(heading_true_rad),
        ..SelectedTargets::default()
    }
}

fn altitude(altitude_m: f64) -> SelectedTargets {
    SelectedTargets {
        altitude_m: Some(altitude_m),
        ..SelectedTargets::default()
    }
}

fn airspeed(airspeed_mps: f64) -> SelectedTargets {
    SelectedTargets {
        airspeed_mps: Some(airspeed_mps),
        ..SelectedTargets::default()
    }
}

fn heading_altitude(output: &MissionOutput) -> HeadingAltitudeIntent {
    match output.intent {
        Some(ControlIntent::HeadingAltitude(demand)) => demand,
        ref other => panic!("a heading and altitude demand, not {other:?}"),
    }
}

/// The horizontal speed and the vertical rate, up positive, of a
/// velocity demand, m/s.
fn velocity(output: &MissionOutput) -> (f64, f64) {
    match output.intent {
        Some(ControlIntent::Velocity(demand)) => (
            f64::from(demand.vx).hypot(f64::from(demand.vy)),
            -f64::from(demand.vz),
        ),
        ref other => panic!("a velocity demand, not {other:?}"),
    }
}

/// The angle from `from` to `to`, radians, in `[-π, π]`.
fn turn(from: f64, to: f64) -> f64 {
    (to - from).sin().atan2((to - from).cos())
}

/// Flies until the vertical mode holds `selected_m`, and then until the
/// aircraft is at it: the capture window is wider than the altitude
/// error the demands then remove.
fn until_altitude_held(rig: &mut Rig, selected_m: f64) {
    rig.step_until(4_000, |rig, _| {
        rig.engine.vertical_status().mode == VerticalMode::Hold
    });
    assert_eq!(rig.engine.vertical_status().mode, VerticalMode::Hold);
    rig.step_until(4_000, |rig, _| (rig.altitude_m() - selected_m).abs() < 5.0);
    assert!((rig.altitude_m() - selected_m).abs() < 5.0);
    assert_eq!(rig.engine.vertical_status().mode, VerticalMode::Hold);
}

#[test]
fn a_selected_heading_is_flown_in_place_of_the_plans_path() {
    for feed in [Feed::Truth, Feed::Navigator] {
        let mut rig = Rig::new(feed);
        rig.fly_to_route();
        let selected = 0.05;
        rig.engine.select(heading(selected)).expect("selects");
        let output = rig.step();
        let demand = heading_altitude(&output);
        assert!(turn(f64::from(demand.heading_rad), selected).abs() < 1e-6);
        // The plan's altitude, to the height of the anchor plane above
        // the ellipsoid there.
        assert!((f64::from(demand.altitude_m) - ALTITUDE_M).abs() < 1.0);
        assert_eq!(rig.engine.lateral_status().mode, LateralMode::HeadingSelect);
        assert_eq!(rig.engine.navigation_display().selected, heading(selected));
    }
}

#[test]
fn a_cleared_heading_returns_to_the_plan() {
    let mut rig = Rig::new(Feed::Truth);
    rig.fly_to_route();
    rig.engine.select(heading(0.05)).expect("selects");
    rig.step_until(400, |_, _| false);
    rig.engine
        .select(SelectedTargets::default())
        .expect("clears");
    let output = rig.step();
    assert!(
        matches!(output.intent, Some(ControlIntent::Velocity(_))),
        "the plan's guidance is flown again: {:?}",
        output.intent
    );
    assert_ne!(rig.engine.lateral_status().mode, LateralMode::HeadingSelect);
}

#[test]
fn a_selected_altitude_is_captured_and_held_on_the_velocity_path() {
    let mut rig = Rig::new(Feed::Truth);
    rig.fly_to_route();
    let selected_m = ALTITUDE_M + 120.0;
    rig.engine.select(altitude(selected_m)).expect("selects");
    let output = rig.step();
    let status = rig.engine.vertical_status();
    assert_eq!(status.mode, VerticalMode::Climb);
    assert_eq!(status.target_m, Some(selected_m));
    let (_, up_mps) = velocity(&output);
    assert!(up_mps > 0.5, "the route's intent climbs: {up_mps} m/s");
    until_altitude_held(&mut rig, selected_m);
    // A lower selection is a descent to capture.
    rig.engine.select(altitude(ALTITUDE_M)).expect("selects");
    let output = rig.step();
    assert_eq!(rig.engine.vertical_status().mode, VerticalMode::Descend);
    assert!(velocity(&output).1 < -0.5);
}

#[test]
fn a_selected_altitude_is_in_each_heading_and_altitude_demand() {
    let selected_m = ALTITUDE_M + 120.0;
    // The engine on the navigator steers the route by heading and
    // altitude; the engine on truth does so for a selected heading.
    for (feed, targets) in [
        (Feed::Navigator, altitude(selected_m)),
        (
            Feed::Truth,
            SelectedTargets {
                heading_true_rad: Some(0.0),
                ..altitude(selected_m)
            },
        ),
    ] {
        let mut rig = Rig::new(feed);
        rig.fly_to_route();
        rig.engine.select(targets).expect("selects");
        let output = rig.step();
        let demand = heading_altitude(&output);
        assert!((f64::from(demand.altitude_m) - selected_m).abs() < 1e-3);
        assert_eq!(rig.engine.vertical_status().mode, VerticalMode::Climb);
        until_altitude_held(&mut rig, selected_m);
    }
}

#[test]
fn a_selected_airspeed_is_flown_within_the_ceiling_on_the_velocity_path() {
    let mut rig = Rig::new(Feed::Truth);
    rig.fly_to_route();
    assert!((velocity(&rig.step()).0 - CRUISE_MPS).abs() < 0.5);
    for (selected, flown) in [(60.0, 60.0), (48.0, 48.0), (90.0, CEILING_MPS)] {
        rig.engine.select(airspeed(selected)).expect("selects");
        let (speed_mps, _) = velocity(&rig.step());
        assert!(
            (speed_mps - flown).abs() < 0.5,
            "{selected} m/s selected, {speed_mps} m/s commanded"
        );
    }
    rig.engine
        .select(SelectedTargets::default())
        .expect("clears");
    assert!((velocity(&rig.step()).0 - CRUISE_MPS).abs() < 0.5);
}

#[test]
fn a_selected_airspeed_is_in_each_heading_and_altitude_demand() {
    for feed in [Feed::Truth, Feed::Navigator] {
        let mut rig = Rig::new(feed);
        rig.fly_to_route();
        for (selected, flown) in [(60.0, 60.0_f32), (48.0, 48.0), (90.0, CEILING_MPS as f32)] {
            let targets = SelectedTargets {
                heading_true_rad: Some(0.0),
                ..airspeed(selected)
            };
            rig.engine.select(targets).expect("selects");
            let demand = heading_altitude(&rig.step());
            assert_eq!(demand.airspeed_mps, Some(flown));
        }
    }
}

#[test]
fn the_engine_refuses_a_selection_it_cannot_fly() {
    let mut rig = Rig::new(Feed::Truth);
    rig.engine.select(heading(1.0)).expect("selects");
    for (targets, target) in [
        (heading(f64::NAN), "heading"),
        (altitude(f64::INFINITY), "altitude"),
        (airspeed(f64::NAN), "airspeed"),
    ] {
        assert!(matches!(
            rig.engine.select(targets),
            Err(SelectError::NotFinite { target: named, .. }) if named == target
        ));
    }
    assert_eq!(
        rig.engine.select(airspeed(0.0)),
        Err(SelectError::AirspeedNotPositive { value: 0.0 })
    );
    assert_eq!(rig.engine.selected(), heading(1.0), "the selection stands");

    let mut config = rig::config(Feed::Truth);
    config.vehicle_class = VehicleClass::Multicopter;
    assert!(config.can_stop());
    let (mut engine, _) = MissionEngine::from_waypoints(&rig::plan(), config).expect("builds");
    assert_eq!(
        engine.select(heading(1.0)),
        Err(SelectError::VehicleCanStop)
    );
    assert_eq!(engine.select(SelectedTargets::default()), Ok(()));
    assert_eq!(engine.selected(), SelectedTargets::default());
}

#[test]
fn a_selected_heading_is_kept_in_one_turn() {
    for (given, kept) in [
        (-FRAC_PI_2, 3.0 * FRAC_PI_2),
        (TAU, 0.0),
        (TAU + 1.0, 1.0),
        (-1e-20, 0.0),
        (PI, PI),
    ] {
        let checked = heading(given).checked(false).expect("a finite heading");
        let kept_rad = checked.heading_true_rad.expect("a heading");
        assert!((kept_rad - kept).abs() < 1e-12, "{given} gives {kept_rad}");
        assert!((0.0..TAU).contains(&kept_rad));
    }
}

#[test]
fn a_change_of_the_selection_is_reported_once() {
    let mut rig = Rig::new(Feed::Truth);
    rig.fly_to_route();
    let targets = SelectedTargets {
        heading_true_rad: Some(1.0),
        altitude_m: Some(ALTITUDE_M + 50.0),
        airspeed_mps: Some(50.0),
    };
    rig.engine.select(targets).expect("selects");
    rig.engine.select(targets).expect("selects again");
    let output = rig.step();
    let reported: Vec<_> = output
        .events
        .iter()
        .filter(|event| matches!(event, MissionEvent::TargetsSelected { .. }))
        .collect();
    assert_eq!(reported, [&MissionEvent::TargetsSelected { targets }]);
    let output = rig.step();
    assert!(
        !output
            .events
            .iter()
            .any(|event| matches!(event, MissionEvent::TargetsSelected { .. }))
    );
}

#[test]
fn a_selection_applies_only_in_flight_under_the_mission() {
    let mut rig = Rig::new(Feed::Navigator);
    let targets = SelectedTargets {
        heading_true_rad: Some(1.0),
        altitude_m: Some(ALTITUDE_M + 300.0),
        airspeed_mps: Some(48.0),
    };
    rig.engine.select(targets).expect("selects before the arm");
    let mut flown = false;
    for _ in 0..400 {
        let output = rig.step();
        let mode = rig.engine.lateral_status().mode;
        if matches!(
            output.state,
            MissionState::AwaitSolution | MissionState::Arming
        ) {
            assert_ne!(mode, LateralMode::HeadingSelect);
            if let Some(ControlIntent::HeadingAltitude(demand)) = output.intent {
                assert!((f64::from(demand.altitude_m) - ALTITUDE_M).abs() < 1.0);
                assert_eq!(demand.airspeed_mps, Some(CRUISE_MPS as f32));
            }
        } else {
            flown = true;
            assert_eq!(mode, LateralMode::HeadingSelect);
            assert_eq!(heading_altitude(&output).airspeed_mps, Some(48.0));
        }
    }
    assert!(flown, "the mission flew");
    // A human holds the motion scope: the engine commands nothing.
    rig.engine.hold(MonotonicNanos::from_nanos(60_000_000_000));
    let output = rig.step();
    assert_eq!(output.state, MissionState::Held);
    assert_eq!(output.intent, None);
}

#[test]
fn a_lost_position_wins_over_a_selected_heading() {
    let mut rig = Rig::new(Feed::Navigator);
    rig.fly_to_route();
    let selected = 0.4;
    rig.engine.select(heading(selected)).expect("selects");
    rig.step_until(600, |rig, _| turn(rig.heading_rad, selected).abs() < 0.01);
    rig.position_known = false;
    let output = rig.step_until(100, |rig, _| {
        rig.engine.lateral_status().mode == LateralMode::HeadingHold
    });
    assert_eq!(rig.engine.lateral_status().mode, LateralMode::HeadingHold);
    // The bounded hold keeps the heading last commanded: the selected one.
    let demand = heading_altitude(&output);
    assert!(turn(f64::from(demand.heading_rad), selected).abs() < 0.05);
    rig.position_known = true;
    rig.step();
    assert_eq!(rig.engine.lateral_status().mode, LateralMode::HeadingSelect);
}

#[test]
fn after_the_plan_a_selection_leaves_the_pattern_and_a_clear_enters_it_again() {
    let mut rig = Rig::new(Feed::Truth);
    rig.step_until(20_000, |rig, _| rig.engine.holding_pattern().is_some());
    assert_eq!(rig.engine.state(), MissionState::AfterPlan);
    let started = |rig: &Rig| {
        rig.events
            .iter()
            .filter(|event| matches!(event, MissionEvent::HoldingPatternStarted { .. }))
            .count()
    };
    assert_eq!(started(&rig), 1);
    let selected_m = ALTITUDE_M + 80.0;
    let targets = SelectedTargets {
        heading_true_rad: Some(PI),
        ..altitude(selected_m)
    };
    rig.engine.select(targets).expect("selects");
    let output = rig.step();
    let demand = heading_altitude(&output);
    assert!(turn(f64::from(demand.heading_rad), PI).abs() < 1e-6);
    assert!((f64::from(demand.altitude_m) - selected_m).abs() < 1e-3);
    assert_eq!(rig.engine.holding_pattern(), None);
    assert_eq!(rig.engine.vertical_status().mode, VerticalMode::Climb);
    until_altitude_held(&mut rig, selected_m);
    rig.engine
        .select(SelectedTargets::default())
        .expect("clears");
    rig.step();
    assert!(rig.engine.holding_pattern().is_some());
    assert_eq!(started(&rig), 2, "the pattern is entered again");
}
