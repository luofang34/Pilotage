#![cfg(test)]

//! A route that runs straight into its last waypoint, fly-by or fly-over,
//! and the holding pattern that follows, closed loop with Aviate's C172
//! lateral law at 60 m/s: in still air and in a 12 m/s crosswind, on the
//! onboard navigator and on simulation truth. The aircraft flies over the
//! last waypoint before it turns, every outbound leg ends, the racetrack
//! stays in its area for three circuits, and the racetrack flown is the
//! one planned.

#[path = "holding_pattern/aircraft.rs"]
mod aircraft;

use aircraft::{ALTITUDE_M, Aircraft, CLOCK, STEP_NANOS, STEP_S, anchor};
use navigate_contract::MonotonicNanos;
use navigate_geodesy::NedOffset;
use navigate_geodesy::wgs84::tangent_plane;
use pilotage_mission::{
    ExecutionTarget, HoldingLeg, MissionConfig, MissionEngine, MissionEvent, MissionState,
    OwnshipSample, OwnshipSource, PlannedWaypoint, TruthRole, WaypointPlan, WaypointTurn,
};
use pilotage_protocol::{ControlAction, VehicleClass};

/// The route: 10 km legs north-west, straight into the last waypoint.
const ROUTE_NED: [[f64; 2]; 3] = [[0.0, 0.0], [7_071.0, -7_071.0], [14_142.0, -14_142.0]];
const CRUISE_IAS_MPS: f64 = 60.0;

fn config(navigator: bool) -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = if navigator {
        OwnshipSource::onboard_navigation()
    } else {
        OwnshipSource::SimulationTruth
    };
    config.execution_target = ExecutionTarget::Simulator;
    config.vehicle_class = VehicleClass::FixedWing;
    config.cruise_height_m = 0.0;
    config.cruise_mps = CRUISE_IAS_MPS;
    config.limits.max_horizontal_mps = 65.0;
    config.limits.max_vertical_mps = 5.0;
    config.limits.max_yaw_rate_rps = 0.2;
    config.route_bank_deg = 22.0;
    config
}

/// The route, its last waypoint of `last_turn`.
fn plan(last_turn: WaypointTurn) -> WaypointPlan {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let mut waypoints: Vec<PlannedWaypoint> = ROUTE_NED
        .iter()
        .map(|[north, east]| {
            let p = plane.from_ned(&NedOffset::new(*north, *east, 0.0));
            PlannedWaypoint::new(
                p.latitude_rad.to_degrees(),
                p.longitude_rad.to_degrees(),
                ALTITUDE_M,
            )
        })
        .collect();
    if let Some(last) = waypoints.last_mut() {
        last.turn = last_turn;
    }
    WaypointPlan::new(waypoints)
}

/// One tick of the flight after the plan completed: the time, where the
/// aircraft is relative to the last waypoint (along the final course, and
/// right of it), and the part of the pattern flown.
struct Held {
    t_s: f64,
    along_m: f64,
    right_m: f64,
    leg: Option<HoldingLeg>,
    circuits: u32,
    turn_radius_m: f64,
}

/// Flies the route and `hold_s` of the pattern after it.
fn fly(
    navigator: bool,
    wind_mps: [f64; 2],
    hold_s: f64,
    last_turn: WaypointTurn,
) -> (Vec<Held>, Vec<MissionEvent>) {
    let (mut engine, _) =
        MissionEngine::from_waypoints(&plan(last_turn), config(navigator)).expect("builds");
    let mut aircraft = Aircraft::new([-500.0, 500.0, 0.0], -45.0_f64.to_radians(), wind_mps);
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let (mut held, mut events) = (Vec::new(), Vec::new());
    let [_, _, [fix_n, fix_e]] = ROUTE_NED;
    let (sin, cos) = (-45.0_f64).to_radians().sin_cos();
    for sequence in 0..60_000_u32 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        if navigator {
            engine.on_onboard_solution(&aircraft.solution(now, sequence), now);
        } else {
            let sample = OwnshipSample {
                ned: aircraft.ned,
                ned_velocity: aircraft.ground_velocity(),
                yaw_rad: Some(aircraft.heading_rad),
                role: TruthRole::SimulationTruth,
                acquired_at: now,
                sequence,
            };
            engine.on_ownship(&sample, now);
        }
        let output = engine.tick(now);
        if let Some(action) = output.action {
            assert_eq!(action.action, ControlAction::Arm);
            engine.on_action_result(action.action_id, true);
        }
        events.extend(output.events.iter().cloned());
        if output.state == MissionState::Complete {
            let (north, east) = (aircraft.ned[0] - fix_n, aircraft.ned[1] - fix_e);
            let status = engine.holding_pattern();
            held.push(Held {
                t_s: held.len() as f64 * STEP_S,
                along_m: north * cos + east * sin,
                right_m: -north * sin + east * cos,
                leg: status.as_ref().map(|status| status.leg),
                circuits: status.as_ref().map_or(0, |status| status.circuits),
                turn_radius_m: status
                    .as_ref()
                    .map_or(f64::NAN, |status| status.pattern.turn_radius_m),
            });
            if held.len() as f64 * STEP_S >= hold_s {
                break;
            }
        }
        aircraft.fly(output.intent.as_ref());
    }
    (held, events)
}

/// The legs flown in order, with the time each started.
fn legs(held: &[Held]) -> Vec<(HoldingLeg, f64)> {
    let mut legs: Vec<(HoldingLeg, f64)> = Vec::new();
    for tick in held {
        let Some(leg) = tick.leg else { continue };
        if legs.last().is_none_or(|(last, _)| *last != leg) {
            legs.push((leg, tick.t_s));
        }
    }
    legs
}

#[test]
fn a_straight_in_last_waypoint_of_either_kind_is_flown_over_and_held_at() {
    let cases = [WaypointTurn::FlyBy, WaypointTurn::FlyOver]
        .into_iter()
        .flat_map(|turn| [true, false].map(|navigator| (turn, navigator)));
    for (turn, navigator) in cases {
        for wind in [[0.0, 0.0], [8.485, 8.485], [-8.485, -8.485]] {
            let (held, events) = fly(navigator, wind, 1_200.0, turn);
            let summary = format!(
                "{turn:?}, navigator {navigator}, wind {wind:?}: {:?}",
                legs(&held)
            );
            // The entry turn starts over the fix, not short of it.
            let entry = held
                .iter()
                .find(|tick| matches!(tick.leg, Some(HoldingLeg::Entry(_))))
                .expect("the pattern was entered");
            assert!(
                entry.along_m >= -1.0,
                "turned {:.0} m short: {summary}",
                -entry.along_m
            );
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, MissionEvent::HoldingPatternEntered { .. }))
            );
            // Every outbound leg ends, within its longest time.
            let flown = legs(&held);
            for pair in flown.windows(2) {
                if pair[0].0 == HoldingLeg::Outbound {
                    let lasted = pair[1].1 - pair[0].1;
                    assert!(
                        lasted <= 3.0 * 60.0 + 1.0,
                        "an outbound leg of {lasted:.0} s: {summary}"
                    );
                }
            }
            assert!(
                held.last().is_some_and(|tick| tick.circuits >= 3),
                "{summary}"
            );
            // The racetrack stays behind the fix and on the holding side:
            // a 1-minute leg at up to 72 m/s over the ground plus two turn
            // radii of at most 1.6 km, with a margin.
            for tick in &held {
                assert!(
                    tick.along_m > -9_000.0 && tick.along_m < 2_500.0,
                    "{summary}"
                );
                assert!(tick.right_m > -800.0 && tick.right_m < 4_000.0, "{summary}");
            }
        }
    }
}

/// How far the racetrack flown after the first circuit strays from the
/// planned one, metres: the turn at the fix past the outbound line, the
/// outbound leg outside it, and the inbound leg across to the non-holding
/// side.
fn strays(held: &[Held]) -> [f64; 3] {
    let settled = held.iter().filter(|tick| tick.circuits >= 1);
    settled.fold([0.0_f64; 3], |worst, tick| {
        let outbound_line = 2.0 * tick.turn_radius_m;
        match tick.leg {
            Some(HoldingLeg::FixTurn) => [
                worst[0].max(tick.right_m - outbound_line),
                worst[1],
                worst[2],
            ],
            Some(HoldingLeg::Outbound) => [
                worst[0],
                worst[1].max(tick.right_m - outbound_line),
                worst[2],
            ],
            Some(HoldingLeg::Inbound) => [worst[0], worst[1], worst[2].max(-tick.right_m)],
            _ => worst,
        }
    })
}

#[test]
fn the_racetrack_flown_is_the_racetrack_planned() {
    for navigator in [true, false] {
        for wind in [[0.0, 0.0], [8.485, 8.485]] {
            let (held, _) = fly(navigator, wind, 1_500.0, WaypointTurn::FlyBy);
            let worst = strays(&held);
            assert!(
                worst.iter().all(|stray| *stray <= 30.0),
                "navigator {navigator}, wind {wind:?}: turn wide by {:.0} m, outbound outside \
                 by {:.0} m, inbound across by {:.0} m",
                worst[0],
                worst[1],
                worst[2]
            );
        }
    }
}
