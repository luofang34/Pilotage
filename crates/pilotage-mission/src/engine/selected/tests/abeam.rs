//! The plan under a selected heading: the active waypoint sequences when
//! the aircraft goes abeam of it near the route, and only then.

use core::f64::consts::FRAC_PI_2;

use navigate_route::{HoldStage, SequenceReason};
use pilotage_protocol::ControlIntent;

use super::rig::{self, Feed, Rig};
use super::{heading, turn};
use crate::{
    LateralMode, MissionEvent, MissionState, PlannedCourseReference, PlannedHold,
    PlannedHoldLength, PlannedPath, SelectedTargets, WaypointPlan, WaypointTurnDirection,
};

/// A route north along the meridian of the anchor, then east: north and
/// east of the anchor, metres. The turn at the third waypoint has a
/// fly-by lead distance.
const ROUTE: [[f64; 2]; 4] = [
    [2_000.0, 0.0],
    [6_000.0, 0.0],
    [10_000.0, 0.0],
    [10_000.0, 4_000.0],
];

fn route() -> WaypointPlan {
    WaypointPlan::new(ROUTE.into_iter().map(rig::waypoint).collect())
}

/// A rig on a selected heading of north, `east_m` beside the route.
fn beside(feed: Feed, east_m: f64, bound_m: Option<f64>, plan: &WaypointPlan) -> Rig {
    let mut config = rig::config(feed);
    if let Some(bound_m) = bound_m {
        config.selected_heading_sequence_bound_m = bound_m;
    }
    let mut rig = Rig::with(feed, config, plan, east_m);
    rig.engine.select(heading(0.0)).expect("selects");
    rig
}

/// Flies until the aircraft is `north_m` north of the anchor. Gives each
/// leg advance: the leg made active, the cross-track distance of the
/// waypoint passed, and how far north the aircraft was, metres.
fn fly_north(rig: &mut Rig, north_m: f64) -> Vec<(usize, f64, f64)> {
    let mut advances = Vec::new();
    for _ in 0..8_000 {
        let output = rig.step();
        for event in &output.events {
            if let MissionEvent::LegAdvanced { to_index, reason } = event {
                let SequenceReason::Passed { cross_track_m } = reason else {
                    panic!("sequenced for {reason:?}");
                };
                advances.push((*to_index, *cross_track_m, rig.ned[0]));
            }
        }
        if rig.ned[0] > north_m {
            break;
        }
    }
    assert!(rig.ned[0] > north_m, "the aircraft flew north");
    advances
}

#[test]
fn a_waypoint_abeam_near_the_route_is_sequenced_as_the_aircraft_passes_it() {
    for feed in [Feed::Truth, Feed::Navigator] {
        let mut rig = beside(feed, 2_000.0, None, &route());
        let advances = fly_north(&mut rig, 11_000.0);
        let legs: Vec<usize> = advances.iter().map(|(leg, ..)| *leg).collect();
        assert_eq!(legs, [1, 2, 3], "{advances:?}");
        for (leg, cross_track_m, north_m) in advances {
            assert!((cross_track_m - 2_000.0).abs() < 50.0, "{cross_track_m} m");
            // On the abeam crossing: the capture radius and the turn
            // ahead of the third waypoint do not sequence before it.
            let abeam_m = ROUTE[leg - 1][0];
            assert!(
                (abeam_m..abeam_m + 20.0).contains(&north_m),
                "leg {leg} became active {north_m} m north"
            );
        }
        assert_eq!(rig.engine.plan_progress(), (3, false));
    }
}

#[test]
fn the_last_waypoint_is_not_sequenced_and_the_plan_does_not_complete() {
    let mut rig = beside(Feed::Truth, 2_000.0, None, &route());
    fly_north(&mut rig, 10_500.0);
    assert_eq!(rig.engine.plan_progress(), (3, false));
    // East along the last leg and past its end.
    rig.engine.select(heading(FRAC_PI_2)).expect("selects");
    let events = rig.events.len();
    rig.step_until(4_000, |rig, _| rig.ned[1] > ROUTE[3][1] + 1_500.0);
    assert!(rig.ned[1] > ROUTE[3][1] + 1_500.0);
    assert!(
        !rig.events[events..].iter().any(|event| matches!(
            event,
            MissionEvent::LegAdvanced { .. }
                | MissionEvent::LastWaypointSequenced { .. }
                | MissionEvent::HoldingPatternStarted { .. }
        )),
        "{:?}",
        &rig.events[events..]
    );
    assert_eq!(rig.engine.plan_progress(), (3, false));
    assert_eq!(rig.engine.state(), MissionState::Enroute);
    assert_eq!(rig.engine.navigation_display().active_waypoint, Some(3));
}

#[test]
fn a_waypoint_abeam_beyond_the_bound_is_not_sequenced() {
    // Beyond the default bound, and beyond a bound the configuration
    // sets.
    for (east_m, bound_m) in [(12_000.0, None), (2_000.0, Some(1_000.0))] {
        let mut rig = beside(Feed::Truth, east_m, bound_m, &route());
        let advances = fly_north(&mut rig, 11_000.0);
        assert_eq!(advances, [], "{east_m} m beside a bound of {bound_m:?}");
        assert_eq!(rig.engine.plan_progress(), (0, false));
    }
    // The same 12 km within a bound the configuration sets.
    let mut rig = beside(Feed::Truth, 12_000.0, Some(15_000.0), &route());
    assert_eq!(fly_north(&mut rig, 11_000.0).len(), 3);
}

#[test]
fn a_cleared_heading_flies_to_the_next_waypoint_not_back_to_the_passed_one() {
    let mut rig = beside(Feed::Truth, 2_000.0, None, &route());
    assert_eq!(fly_north(&mut rig, 3_000.0).len(), 1, "the first is passed");
    rig.engine
        .select(SelectedTargets::default())
        .expect("clears");
    let events = rig.events.len();
    for _ in 0..200 {
        let from = rig.ned;
        let heading_rad = rig.heading_rad;
        let output = rig.step();
        let Some(ControlIntent::Velocity(demand)) = output.intent else {
            panic!("the plan's guidance, not {:?}", output.intent);
        };
        let (sin, cos) = heading_rad.sin_cos();
        let (vx, vy) = (f64::from(demand.vx), f64::from(demand.vy));
        let course = (vx * sin + vy * cos).atan2(vx * cos - vy * sin);
        let back = (ROUTE[0][1] - from[1]).atan2(ROUTE[0][0] - from[0]);
        // North and west onto the second leg; the passed waypoint is
        // south and west.
        assert!((-FRAC_PI_2..=0.0).contains(&course), "course {course} rad");
        assert!(turn(course, back).abs() > 1.0, "course {course} rad");
        assert!(matches!(
            rig.engine.lateral_status().mode,
            LateralMode::Track | LateralMode::DirectTo
        ));
    }
    assert!(
        !rig.events[events..]
            .iter()
            .any(|event| matches!(event, MissionEvent::FixOverflown { .. })),
        "{:?}",
        &rig.events[events..]
    );
    // The plan goes on from the second waypoint.
    rig.step_until(4_000, |rig, _| rig.engine.plan_progress().0 == 2);
    assert_eq!(rig.engine.plan_progress(), (2, false));
}

#[test]
fn a_hold_is_not_sequenced_past() {
    let hold = PlannedHold {
        inbound_course_rad: 0.0,
        reference: PlannedCourseReference::True,
        turn: WaypointTurnDirection::Right,
        length: PlannedHoldLength::DistanceM(900.0),
    };
    for path in [PlannedPath::HoldManual(hold), PlannedPath::HoldToFix(hold)] {
        let plan = WaypointPlan::new(vec![
            rig::waypoint(ROUTE[0]),
            rig::waypoint(ROUTE[1]).with_path(path),
            rig::waypoint(ROUTE[2]),
        ]);
        let mut rig = beside(Feed::Truth, 2_000.0, None, &plan);
        let advances = fly_north(&mut rig, 9_000.0);
        // The hold becomes the active leg, and stays it abeam of its fix.
        let legs: Vec<usize> = advances.iter().map(|(leg, ..)| *leg).collect();
        assert_eq!(legs, [1], "{advances:?}");
        assert_eq!(rig.engine.plan_progress(), (1, false));
        let status = rig.engine.hold_status().expect("the hold is active");
        assert_eq!(status.stage, HoldStage::ToFix);
        assert_eq!(status.circuits_completed, 0);
    }
}
