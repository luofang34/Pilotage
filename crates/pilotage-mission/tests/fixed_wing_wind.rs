#![cfg(test)]

//! A fixed wing flown on velocity demands in a wind. Its flight controller
//! reads a velocity's direction as the course over the ground and its speed
//! as the indicated airspeed, so the engine sends the airspeed that makes
//! good the ground speed it means: a leg flown into a headwind takes about
//! as long as in still air, and a crosswind does not pull the track off
//! the leg.

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use navigate_geodesy::NedOffset;
use navigate_geodesy::wgs84::tangent_plane;
use pilotage_mission::{
    ExecutionTarget, MissionConfig, MissionEngine, MissionEvent, OwnshipSample, OwnshipSource,
    PlannedWaypoint, TruthRole, WaypointPlan, true_airspeed_for,
};
use pilotage_protocol::{ControlAction, ControlIntent, VehicleClass};

const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 50_000_000;
const STEP_S: f64 = 0.05;
const ALTITUDE_M: f64 = 2_000.0;
const CRUISE_IAS_MPS: f64 = 55.0;
const MIN_IAS_MPS: f64 = 33.0;
const MAX_IAS_MPS: f64 = 65.0;
const GRAVITY_MPS2: f64 = 9.806_65;
/// Aviate C172 `route_bank_angle`, `course_time_constant`,
/// `roll_time_constant`, and `max_roll_rate`.
const ROUTE_BANK_RAD: f64 = 0.44;
const COURSE_TIME_CONSTANT_S: f64 = 3.0;
const ROLL_TIME_CONSTANT_S: f64 = 0.5;
const MAX_ROLL_RATE_RPS: f64 = 0.5;
/// The leg the test times: due north, 8 km.
const LEG_M: f64 = 8_000.0;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(40.0_f64.to_radians(), -75.0_f64.to_radians(), ALTITUDE_M)
}

fn config() -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = OwnshipSource::SimulationTruth;
    config.execution_target = ExecutionTarget::Simulator;
    config.vehicle_class = VehicleClass::FixedWing;
    config.cruise_height_m = 0.0;
    config.cruise_mps = CRUISE_IAS_MPS;
    config.limits.max_horizontal_mps = MAX_IAS_MPS;
    config.limits.max_vertical_mps = 5.0;
    config.limits.max_yaw_rate_rps = 0.2;
    config.route_bank_deg = 22.0;
    config
}

fn plan() -> WaypointPlan {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let points = [[0.0, 0.0], [LEG_M, 0.0], [LEG_M, 4_000.0]];
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

/// A fixed wing in a steady `wind` that flies Aviate's course law on its
/// ground course at the indicated airspeed it is commanded.
struct Aircraft {
    ned: [f64; 2],
    heading_rad: f64,
    bank_rad: f64,
    indicated_mps: f64,
    wind: [f64; 2],
}

impl Aircraft {
    fn ground_velocity(&self) -> [f64; 2] {
        let airspeed = true_airspeed_for(self.indicated_mps, ALTITUDE_M);
        [
            airspeed * self.heading_rad.cos() + self.wind[0],
            airspeed * self.heading_rad.sin() + self.wind[1],
        ]
    }

    fn fly(&mut self, demand: [f64; 2]) {
        let speed = demand[0].hypot(demand[1]);
        let course = self.ground_velocity();
        let ground_course = course[1].atan2(course[0]);
        let wanted = if speed > 0.0 {
            self.indicated_mps = speed.clamp(MIN_IAS_MPS, MAX_IAS_MPS);
            demand[1].atan2(demand[0])
        } else {
            ground_course
        };
        let airspeed = true_airspeed_for(self.indicated_mps, ALTITUDE_M);
        let rate_per_tangent = GRAVITY_MPS2 / airspeed;
        let error = (wanted - ground_course)
            .sin()
            .atan2((wanted - ground_course).cos());
        let wanted_bank = (error / COURSE_TIME_CONSTANT_S / rate_per_tangent)
            .atan()
            .clamp(-ROUTE_BANK_RAD, ROUTE_BANK_RAD);
        let roll_rate = ((wanted_bank - self.bank_rad) / ROLL_TIME_CONSTANT_S)
            .clamp(-MAX_ROLL_RATE_RPS, MAX_ROLL_RATE_RPS);
        self.bank_rad += roll_rate * STEP_S;
        self.heading_rad += rate_per_tangent * self.bank_rad.tan() * STEP_S;
    }

    fn step(&mut self) {
        let velocity = self.ground_velocity();
        self.ned[0] += velocity[0] * STEP_S;
        self.ned[1] += velocity[1] * STEP_S;
    }
}

/// The time the first leg took, seconds, and the farthest the track
/// strayed from it in its middle, metres.
fn fly_first_leg(wind: [f64; 2]) -> (f64, f64) {
    let (mut engine, _) = MissionEngine::from_waypoints(&plan(), config()).expect("builds");
    let mut aircraft = Aircraft {
        ned: [-1_500.0, 0.0],
        heading_rad: 0.0,
        bank_rad: 0.0,
        indicated_mps: CRUISE_IAS_MPS,
        wind,
    };
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let (mut leg_start, mut leg_end) = (None, None);
    let mut worst_m: f64 = 0.0;
    for tick in 0..20_000_u32 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        let velocity = aircraft.ground_velocity();
        let sample = OwnshipSample {
            ned: [aircraft.ned[0], aircraft.ned[1], 0.0],
            ned_velocity: [velocity[0], velocity[1], 0.0],
            yaw_rad: Some(aircraft.heading_rad),
            role: TruthRole::SimulationTruth,
            acquired_at: now,
            sequence: tick,
        };
        engine.on_ownship(&sample, now);
        let output = engine.tick(now);
        if let Some(action) = output.action {
            assert_eq!(action.action, ControlAction::Arm);
            engine.on_action_result(action.action_id, true);
        }
        for event in &output.events {
            if let MissionEvent::LegAdvanced { to_index, .. } = event {
                match to_index {
                    1 => leg_start = Some(tick),
                    2 => leg_end = Some(tick),
                    _ => {}
                }
            }
        }
        if leg_end.is_some() {
            break;
        }
        if leg_start.is_some() && (2_000.0..6_000.0).contains(&aircraft.ned[0]) {
            worst_m = worst_m.max(aircraft.ned[1].abs());
        }
        if let Some(ControlIntent::Velocity(v)) = output.intent {
            let (sin, cos) = aircraft.heading_rad.sin_cos();
            let (vx, vy) = (f64::from(v.vx), f64::from(v.vy));
            aircraft.fly([vx * cos - vy * sin, vx * sin + vy * cos]);
        }
        aircraft.step();
    }
    let (Some(start), Some(end)) = (leg_start, leg_end) else {
        panic!("the first leg was not flown in {wind:?}");
    };
    (f64::from(end - start) * STEP_S, worst_m)
}

#[test]
fn a_headwind_and_a_crosswind_are_made_good_over_the_ground() {
    let (still_s, still_m) = fly_first_leg([0.0, 0.0]);
    // 12 m/s on the nose, and 12 m/s from the left.
    let (headwind_s, headwind_m) = fly_first_leg([-12.0, 0.0]);
    let (crosswind_s, crosswind_m) = fly_first_leg([0.0, 12.0]);
    let summary = format!(
        "still air {still_s:.1} s / {still_m:.1} m, headwind {headwind_s:.1} s / \
         {headwind_m:.1} m, crosswind {crosswind_s:.1} s / {crosswind_m:.1} m"
    );
    // Uncorrected, the headwind costs a fifth of the ground speed and the
    // leg takes 30 % longer (154 s against 118 s). Made good, it takes 3 %
    // longer: the airspeed it needs is capped at 65 m/s indicated.
    assert!(headwind_s < 1.05 * still_s, "{summary}");
    assert!(crosswind_s < 1.03 * still_s, "{summary}");
    assert!(
        headwind_m < 10.0 && crosswind_m < 30.0 && still_m < 10.0,
        "{summary}"
    );
}
