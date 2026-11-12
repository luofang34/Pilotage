#![cfg(test)]

//! A fixed wing keeps its legs with no standing cross-track error, closed
//! loop with Aviate's C172 lateral law, whatever its flight controller's
//! heading reference reads and whatever the wind. Each case flies 12 km
//! north and then 12 km east, with the flight controller's references
//! biased 2° either way and a 12 m/s wind across the first leg from either
//! side: on the onboard navigator, which steers by heading, and on
//! simulation truth, which steers by velocity. Late on each leg the track
//! is within 5 m of the leg, and the turn onto the second leg swings no
//! more than 30 m past it.

#[path = "track_keeping/aircraft.rs"]
mod aircraft;

use aircraft::Aircraft;
use navigate_contract::{
    AttitudeQuaternion, ClockDomainId, Ellipsoid, FaultDetection, GeodeticPosition,
    IntegrityAssessment, MonotonicNanos, NavigationSolution, NedVelocity, Redundancy, SensorClass,
    SolutionQuality, SolutionStamp, SourceComposition, SourceEpoch, SymmetricCov3,
    WrappingSequence,
};
use navigate_geodesy::NedOffset;
use navigate_geodesy::wgs84::tangent_plane;
use pilotage_mission::{
    ExecutionTarget, HoldingLeg, MissionConfig, MissionEngine, MissionState, OwnshipSample,
    OwnshipSource, PlannedWaypoint, TruthRole, WaypointPlan,
};
use pilotage_protocol::{ControlAction, VehicleClass};

const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 50_000_000;
pub(crate) const STEP_S: f64 = 0.05;
pub(crate) const ALTITUDE_M: f64 = 2_000.0;
const LEG_M: f64 = 12_000.0;
/// Where along each leg the track counts as settled, metres from its
/// start: from 7 km, since a turn into a tailwind starts early and rolls
/// out inside the next leg, and on velocity demands the capture from there
/// takes some kilometres; to 9.5 km, short of where the turn ahead of the
/// corner starts in any of these winds.
const SETTLED_ALONG_M: [f64; 2] = [7_000.0, 9_500.0];

/// Where the engine's solution comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Feed {
    Navigator,
    Truth,
}

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(40.0_f64.to_radians(), -75.0_f64.to_radians(), ALTITUDE_M)
}

fn config(feed: Feed) -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = match feed {
        Feed::Navigator => OwnshipSource::onboard_navigation(),
        Feed::Truth => OwnshipSource::SimulationTruth,
    };
    config.execution_target = ExecutionTarget::Simulator;
    config.vehicle_class = VehicleClass::FixedWing;
    config.cruise_height_m = 0.0;
    config.cruise_mps = 55.0;
    config.limits.max_horizontal_mps = 65.0;
    config.limits.max_vertical_mps = 5.0;
    config.limits.max_yaw_rate_rps = 0.2;
    config.route_bank_deg = 22.0;
    config
}

fn plan() -> WaypointPlan {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let points = [[0.0, 0.0], [LEG_M, 0.0], [LEG_M, LEG_M]];
    WaypointPlan::new(
        points
            .iter()
            .map(|[north, east]| {
                let p = plane.from_ned(&NedOffset::new(*north, *east, 0.0));
                PlannedWaypoint::new(
                    p.latitude_rad.to_degrees(),
                    p.longitude_rad.to_degrees(),
                    ALTITUDE_M,
                )
            })
            .collect(),
    )
}

/// The navigator's solution of the aircraft: its true position and ground
/// velocity, and its heading read `yaw_bias_rad` right of the true one.
fn solution(
    aircraft: &Aircraft,
    yaw_bias_rad: f64,
    now: MonotonicNanos,
    sequence: u32,
) -> NavigationSolution {
    const SIGMA_M: f64 = 5.0;
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let position = plane.from_ned(&NedOffset::new(aircraft.ned[0], aircraft.ned[1], 0.0));
    let velocity = aircraft.ground_velocity();
    let composition = SourceComposition::of(SensorClass::VisualLandmark);
    let variance = SIGMA_M * SIGMA_M;
    let half = (aircraft.heading_rad + yaw_bias_rad) / 2.0;
    NavigationSolution::new(
        SolutionStamp::new(
            SourceEpoch::new(0),
            WrappingSequence::new(sequence),
            now,
            CLOCK,
        ),
        position,
        NedVelocity::new(velocity[0], velocity[1], 0.0),
        SymmetricCov3::from_diagonal(variance, variance, variance),
        SymmetricCov3::from_diagonal(0.25, 0.25, 0.25),
        IntegrityAssessment::new(
            SolutionQuality::Good,
            composition,
            Redundancy::None,
            SIGMA_M,
            SIGMA_M,
            FaultDetection::Unavailable,
        ),
        composition,
        Ellipsoid::WGS84,
    )
    .with_attitude(AttitudeQuaternion::new(half.cos(), 0.0, 0.0, half.sin()))
}

/// The track of one flight in the anchor's NED plane, until the plan
/// completes. The navigator's heading carries the flight controller's
/// bias when it `shares_bias`, as one read from the same reference does.
fn fly(feed: Feed, wind_mps: [f64; 2], bias_deg: f64, shares_bias: bool) -> Vec<[f64; 2]> {
    fly_and_hold(feed, wind_mps, bias_deg, shares_bias, 0.0).track
}

/// A flight: the track in the anchor's NED plane, the holding-pattern leg
/// at each step, and the pattern's turn radius.
struct Flight {
    track: Vec<[f64; 2]>,
    legs: Vec<Option<HoldingLeg>>,
    turn_radius_m: f64,
}

/// A flight until the plan completes and then `hold_s` in the holding
/// pattern.
fn fly_and_hold(
    feed: Feed,
    wind_mps: [f64; 2],
    bias_deg: f64,
    shares_bias: bool,
    hold_s: f64,
) -> Flight {
    let yaw_bias_rad = if shares_bias {
        bias_deg.to_radians()
    } else {
        0.0
    };
    let (mut engine, _) = MissionEngine::from_waypoints(&plan(), config(feed)).expect("builds");
    let mut aircraft = Aircraft::new([-1_500.0, 0.0], 0.0, wind_mps, bias_deg.to_radians());
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut flight = Flight {
        track: Vec::new(),
        legs: Vec::new(),
        turn_radius_m: f64::NAN,
    };
    let mut held_steps = 0.0;
    for sequence in 0..60_000_u32 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        match feed {
            Feed::Navigator => {
                engine.on_onboard_solution(&solution(&aircraft, yaw_bias_rad, now, sequence), now);
            }
            Feed::Truth => {
                let velocity = aircraft.ground_velocity();
                let sample = OwnshipSample {
                    ned: [aircraft.ned[0], aircraft.ned[1], 0.0],
                    ned_velocity: [velocity[0], velocity[1], 0.0],
                    yaw_rad: Some(aircraft.heading_rad),
                    role: TruthRole::SimulationTruth,
                    acquired_at: now,
                    sequence,
                };
                engine.on_ownship(&sample, now);
            }
        }
        let output = engine.tick(now);
        if let Some(action) = output.action {
            assert_eq!(action.action, ControlAction::Arm);
            engine.on_action_result(action.action_id, true);
        }
        let pattern = engine.holding_pattern();
        flight.track.push(aircraft.ned);
        flight.legs.push(pattern.as_ref().map(|status| status.leg));
        if let Some(status) = pattern {
            flight.turn_radius_m = status.pattern.turn_radius_m;
        }
        if output.state == MissionState::AfterPlan {
            held_steps += 1.0;
            if held_steps * STEP_S >= hold_s {
                break;
            }
        }
        aircraft.fly(output.intent.as_ref());
    }
    flight
}

/// The cross-track error late on each leg, the largest absolute one in
/// its settled stretch, metres: right of the first leg (east), and right
/// of the second (south).
fn settled_cross_track_m(track: &[[f64; 2]]) -> [f64; 2] {
    let first = track
        .iter()
        .filter(|p| {
            (SETTLED_ALONG_M[0]..SETTLED_ALONG_M[1]).contains(&p[0]) && p[1].abs() < 1_000.0
        })
        .map(|p| p[1].abs())
        .fold(f64::NAN, f64::max);
    let second = track
        .iter()
        .filter(|p| {
            (SETTLED_ALONG_M[0]..SETTLED_ALONG_M[1]).contains(&p[1]) && p[0] > LEG_M - 1_000.0
        })
        .map(|p| (LEG_M - p[0]).abs())
        .fold(f64::NAN, f64::max);
    [first, second]
}

/// How far the turn onto the second leg swings past it, metres: north of
/// the eastbound leg, once the aircraft is east of the corner's turn.
fn overshoot_m(track: &[[f64; 2]]) -> f64 {
    track
        .iter()
        .filter(|p| p[1] > 500.0 && p[1] < SETTLED_ALONG_M[1])
        .map(|p| p[0] - LEG_M)
        .fold(0.0, f64::max)
}

/// How far the turn onto the second leg rolls out inside it, metres:
/// south of the eastbound leg, once the aircraft is past the turn.
fn inside_miss_m(track: &[[f64; 2]]) -> f64 {
    track
        .iter()
        .filter(|p| p[0] > LEG_M - 3_000.0 && p[1] > PAST_TURN_M && p[1] < SETTLED_ALONG_M[1])
        .map(|p| LEG_M - p[0])
        .fold(0.0, f64::max)
}

/// How far east of the corner a turn onto the second leg has ended in any
/// of these winds, metres.
const PAST_TURN_M: f64 = 1_500.0;

/// The most a turn may roll out inside the next leg, metres: a turn into
/// a tailwind is sized for the speed it ends at, and starts early when
/// the wind is behind the next leg.
const MAX_INSIDE_M: f64 = 200.0;

/// Flies every wind case at each of `biases_deg` on `feed` and asserts the
/// bounds.
fn assert_kept(feed: Feed, shares_bias: bool, biases_deg: &[f64]) {
    let mut report = Vec::new();
    let mut failed = false;
    for wind in [[0.0, 12.0], [0.0, -12.0], [12.0, 0.0], [-12.0, 0.0]] {
        for bias_deg in biases_deg {
            let track = fly(feed, wind, *bias_deg, shares_bias);
            let settled = settled_cross_track_m(&track);
            let overshoot = overshoot_m(&track);
            let inside = inside_miss_m(&track);
            let fits =
                settled.iter().all(|xtk| *xtk < 5.0) && overshoot <= 30.0 && inside <= MAX_INSIDE_M;
            failed |= !fits;
            report.push(format!(
                "{feed:?} wind {wind:?} bias {bias_deg}°: settled {settled:.1?} m, \
                 overshoot {overshoot:.1} m, inside {inside:.0} m{}",
                if fits { "" } else { "  <- out of bounds" }
            ));
        }
    }
    assert!(!failed, "{}", report.join("\n"));
}

#[test]
fn on_heading_demands_each_leg_is_kept() {
    assert_kept(Feed::Navigator, false, &[2.0, -2.0]);
}

#[test]
fn on_heading_demands_a_bias_the_navigator_also_reads_keeps_each_leg() {
    assert_kept(Feed::Navigator, true, &[2.0, -2.0]);
}

#[test]
fn on_velocity_demands_each_leg_is_kept() {
    assert_kept(Feed::Truth, false, &[2.0, -2.0]);
}

#[test]
fn a_large_bias_is_learned_on_the_first_leg_and_kept_on_the_next() {
    // 8° holds the aircraft 130 m off the track with no trim: well inside
    // the error the trim integrates over.
    let track = fly(Feed::Truth, [0.0, 0.0], 8.0, false);
    let late_first = track
        .iter()
        .filter(|p| (9_000.0..SETTLED_ALONG_M[1]).contains(&p[0]) && p[1].abs() < 1_000.0)
        .map(|p| p[1].abs())
        .fold(f64::NAN, f64::max);
    let [_, second] = settled_cross_track_m(&track);
    let overshoot = overshoot_m(&track);
    assert!(
        late_first < 5.0 && second < 5.0 && overshoot <= 30.0,
        "late on the first leg {late_first:.1} m, settled on the second {second:.1} m, \
         overshoot {overshoot:.1} m"
    );
}

#[test]
fn the_holding_pattern_legs_are_kept_with_a_bias_too() {
    // After the plan, the pattern's inbound leg runs east along the second
    // leg's line into its end, and the outbound leg two turn radii south.
    for feed in [Feed::Navigator, Feed::Truth] {
        let flight = fly_and_hold(feed, [0.0, 0.0], 2.0, false, 900.0);
        let (radius, start) = (flight.turn_radius_m, flight.legs.len() / 2);
        let on = |step: usize, wanted: HoldingLeg| {
            flight.legs.get(step).copied().flatten() == Some(wanted)
        };
        // The last 10 s of each leg flown, after its capture.
        let off = |wanted: HoldingLeg, line_north_m: f64| {
            (start..flight.legs.len())
                .filter(|step| on(*step, wanted) && !on(step + 200, wanted))
                .filter(|step| on(step.saturating_sub(400), wanted))
                .filter_map(|step| flight.track.get(step))
                .map(|p| (p[0] - line_north_m).abs())
                .fold(0.0, f64::max)
        };
        let inbound = off(HoldingLeg::Inbound, LEG_M);
        let outbound = off(HoldingLeg::Outbound, LEG_M - 2.0 * radius);
        // A leg of a minute ends before the capture after the turn onto it
        // has settled; on velocity demands that leaves some metres with no
        // bias at all (5.6 m), against 30 m that a 2° bias holds untrimmed.
        let bound_m = match feed {
            Feed::Navigator => 5.0,
            Feed::Truth => 10.0,
        };
        assert!(
            inbound < bound_m && outbound < bound_m,
            "{feed:?}: inbound {inbound:.1} m, outbound {outbound:.1} m off"
        );
    }
}
