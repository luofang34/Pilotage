//! A holding pattern that loses its solution on the outbound leg: the
//! racetrack goes on by time and heading around the fix for five circuits,
//! never on the outbound heading alone, and the pattern is entered again
//! once the solution comes back.

use pilotage_mission::{HoldingLeg, MissionEvent};

use super::aircraft::STEP_S;
use super::{FIX_NED, Feed, Run};

/// The longest the solution stays lost, seconds: five circuits of two
/// minute legs and two half turns at the standard rate take 20 minutes in
/// still air, and longer with the outbound leg into the wind.
const LOST_S: f64 = 2_400.0;

#[test]
fn a_hold_without_a_solution_circles_the_fix_by_time() {
    for wind in [[0.0, 0.0], [-8.485, 8.485], [12.0, 0.0]] {
        let mut run = Run::new(Feed::Navigator, wind);
        run.fly_and_hold(300.0);
        // On to the outbound leg of a later circuit.
        while run.samples.last().and_then(|s| s.leg) != Some(HoldingLeg::Outbound) {
            run.step(true);
        }
        for _ in 0..40 {
            run.step(true);
        }
        let pattern = run.pattern();
        let lost_at = run.samples.len();
        // Lost until the heading has turned right through five whole turns.
        let (mut turned, mut heading, mut lost_s) = (0.0, run.aircraft.heading_rad, 0.0);
        while turned < 5.0 * core::f64::consts::TAU && lost_s < LOST_S {
            run.step(false);
            let change = run.aircraft.heading_rad - heading;
            turned += change.sin().atan2(change.cos());
            heading = run.aircraft.heading_rad;
            lost_s += STEP_S;
        }
        let lost = run.samples.get(lost_at..).unwrap_or_default();
        assert!(
            lost.iter().all(|s| s.commanded),
            "wind {wind:?}: a tick without a command"
        );
        assert!(
            turned >= 5.0 * core::f64::consts::TAU,
            "wind {wind:?}: turned {:.1} times in {lost_s:.0} s",
            turned / core::f64::consts::TAU
        );
        // Near the fix all along: within half as much again as the
        // racetrack reaches from it.
        let extent_m =
            (pattern.leg_length_m + pattern.turn_radius_m).hypot(2.0 * pattern.turn_radius_m);
        let farthest_m = lost
            .iter()
            .map(|s| (s.ned[0] - FIX_NED[0]).hypot(s.ned[1] - FIX_NED[1]))
            .fold(0.0, f64::max);
        assert!(
            farthest_m <= 1.5 * extent_m,
            "wind {wind:?}: {farthest_m:.0} m from the fix, racetrack reaches {extent_m:.0} m"
        );
        // The solution comes back: the pattern is entered again and flown.
        let entries_before = run.entries().len();
        let back_at = run.samples.len();
        run.fly_and_hold(600.0);
        assert!(
            run.entries().len() > entries_before,
            "wind {wind:?}: not entered again"
        );
        let back = run.samples.get(back_at..).unwrap_or_default();
        let circuits = back.last().map_or(0, |s| s.circuits);
        assert!(
            circuits >= 1,
            "wind {wind:?}: {circuits} circuits after the solution came back"
        );
        assert!(
            run.events
                .iter()
                .any(|e| matches!(e, MissionEvent::NavigationHold { .. }))
        );
    }
}
