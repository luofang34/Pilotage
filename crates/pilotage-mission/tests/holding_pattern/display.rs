//! The navigation display of a flight: the plan, the leg flown, and the
//! holding pattern, as an HSI and a moving map show them.

use pilotage_mission::{ToFrom, WaypointProgress, true_airspeed_for};

use super::aircraft::{ALTITUDE_M, CRUISE_IAS_MPS};
use super::{FIX_NED, Feed, MissionState, Run, Sample};

fn groundspeed_mps() -> f64 {
    true_airspeed_for(CRUISE_IAS_MPS, ALTITUDE_M)
}

#[test]
fn the_navigation_display_shows_the_plan_and_the_leg_flown() {
    let mut run = Run::new(Feed::Navigator, [0.0, 0.0]);
    // No solution yet: the plan is shown, with no leg guidance.
    run.step(false);
    let first = &run.samples[0].display;
    assert_eq!(first.leg, None);
    assert_eq!(first.active_waypoint, Some(0));
    assert_eq!(first.holding_pattern, None);
    run.fly_and_hold(10.0);
    // Mid second leg: from WP1 to WP2, due north.
    let mid = run
        .samples
        .iter()
        .find(|s| s.ned[0] > 3_000.0)
        .expect("the second leg was flown");
    let leg = mid.display.leg.as_ref().expect("leg guidance on the plan");
    assert_eq!(leg.from.as_ref().map(|fix| fix.ident.as_str()), Some("WP1"));
    assert_eq!(leg.to.ident, "WP2");
    assert!(
        leg.desired_track_rad.sin().abs() < 1e-3,
        "{}",
        leg.desired_track_rad
    );
    assert!(
        leg.cross_track_m.is_some_and(|xtk| xtk.abs() < 30.0),
        "{leg:?}"
    );
    assert!(
        (leg.distance_to_go_m - (FIX_NED[0] - mid.ned[0])).abs() < 5.0,
        "{leg:?}"
    );
    let time_to_go = leg.time_to_go_s.expect("a time to go");
    assert!(
        (time_to_go - leg.distance_to_go_m / groundspeed_mps()).abs() < 2.0,
        "{leg:?}"
    );
    assert_eq!(leg.to_from, ToFrom::To);
    assert!(
        mid.guidance
            .as_ref()
            .is_some_and(|guidance| !guidance.holding && guidance.to_from == Some(ToFrom::To)),
        "a plan leg is sequenced, not held, toward its waypoint"
    );
    assert_eq!(mid.display.active_waypoint, Some(1));
    let progress: Vec<WaypointProgress> =
        mid.display.waypoints.iter().map(|w| w.progress).collect();
    assert_eq!(
        progress,
        [WaypointProgress::Flown, WaypointProgress::Active]
    );
}

#[test]
fn in_the_holding_pattern_the_leg_is_the_inbound_course_to_the_fix() {
    let mut run = Run::new(Feed::Navigator, [0.0, 0.0]);
    run.fly_and_hold(600.0);
    let holding: Vec<&Sample> = run
        .samples
        .iter()
        .filter(|s| s.state == MissionState::Complete)
        .collect();
    let mut seen = (false, false);
    for sample in holding {
        let display = &sample.display;
        assert!(display.plan_complete);
        assert_eq!(display.active_waypoint, None);
        assert!(
            display
                .waypoints
                .iter()
                .all(|w| w.progress == WaypointProgress::Flown)
        );
        let Some(status) = display.holding_pattern.as_ref() else {
            continue;
        };
        assert_eq!(status.pattern.fix_ident, "WP2");
        let length = groundspeed_mps() * 60.0;
        assert!(
            (status.pattern.leg_length_m - length).abs() < 1.0,
            "{status:?}"
        );
        let leg = display.leg.as_ref().expect("leg guidance in the pattern");
        assert_eq!(leg.to.ident, "WP2");
        // The published guidance is the same inbound leg, marked a hold.
        let guidance = sample.guidance.as_ref().expect("guidance in the pattern");
        assert!(guidance.holding, "{guidance:?}");
        assert_eq!(guidance.to_ident, "WP2");
        assert_eq!(guidance.course_rad, status.pattern.inbound_course_rad);
        assert_eq!(
            guidance.to_from,
            Some(leg.to_from),
            "the flag flips with the display's"
        );
        assert_eq!(leg.from, None);
        assert_eq!(leg.desired_track_rad, status.pattern.inbound_course_rad);
        // Right of the northbound inbound course is east; the fix is
        // ahead south of it and behind north of it.
        let (x, y) = (sample.ned[0] - FIX_NED[0], sample.ned[1] - FIX_NED[1]);
        assert!(
            leg.cross_track_m.is_some_and(|xtk| (xtk - y).abs() < 1.0),
            "{leg:?} at {y}"
        );
        if x < -200.0 {
            assert_eq!(leg.to_from, ToFrom::To);
            seen.0 = true;
        } else if x > 200.0 {
            assert_eq!(leg.to_from, ToFrom::From);
            seen.1 = true;
        }
    }
    assert_eq!(seen, (true, true));
}
