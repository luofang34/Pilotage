#![cfg(test)]

//! Closed-loop verification of ARINC 424 / CIFP leg types (`IF`, `CF`, `CA`,
//! `DF`), magnetic (`P4`) WMM2025 course resolution, in-route holds (`HA`,
//! `HF`, `HM` with manual exit), and mandatory turn direction (`L` / `R`).

use core::f64::consts::{FRAC_PI_2, PI};

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use navigate_geodesy::NedOffset;
use navigate_geodesy::wgs84::tangent_plane;
use navigate_route::SequenceReason;
use pilotage_mission::{
    ExecutionTarget, HoldStage, MissionBuildError, MissionConfig, MissionEngine, MissionEvent,
    OwnshipSample, OwnshipSource, PlannedAltitude, PlannedCourseReference, PlannedHold,
    PlannedHoldLength, PlannedPath, PlannedWaypoint, TruthRole, WaypointPlan, WaypointTurn,
    WaypointTurnDirection, true_airspeed_for,
};
use pilotage_protocol::{ControlAction, ControlIntent, VehicleClass};
use pilotage_world::MagneticEpoch;

const CLOCK: ClockDomainId = ClockDomainId::new(13);
const STEP_NANOS: u64 = 100_000_000;
const STEP_S: f64 = 0.1;
const BASE_ALT_M: f64 = 400.0;
const IAS_MPS: f64 = 35.0;
const GRAVITY_MPS2: f64 = 9.806_65;
const BANK_LIMIT_RAD: f64 = 0.44;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(39.0_f64.to_radians(), -75.5_f64.to_radians(), BASE_ALT_M)
}

fn wp(north_m: f64, east_m: f64, alt_m: f64, name: &str) -> PlannedWaypoint {
    let plane = tangent_plane(anchor()).expect("anchor");
    let pos = plane.from_ned(&NedOffset::new(north_m, east_m, BASE_ALT_M - alt_m));
    PlannedWaypoint::new(
        pos.latitude_rad.to_degrees(),
        pos.longitude_rad.to_degrees(),
        alt_m,
    )
    .with_name(name)
    .with_turn(WaypointTurn::FlyOver)
}

fn fixed_wing_config(with_epoch: bool) -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = OwnshipSource::SimulationTruth;
    config.execution_target = ExecutionTarget::Simulator;
    config.vehicle_class = VehicleClass::FixedWing;
    config.cruise_height_m = 0.0;
    config.cruise_mps = IAS_MPS;
    config.climb_rate_mps = 4.0;
    config.limits.max_horizontal_mps = 45.0;
    config.limits.max_vertical_mps = 5.0;
    config.route_bank_deg = 22.0;
    if with_epoch {
        config.magnetic_epoch = Some(MagneticEpoch::from_decimal_year(2026.5));
    }
    config
}

struct SimPlane {
    ned: [f64; 3],
    course_rad: f64,
    ias_mps: f64,
    climb_mps: f64,
}

impl SimPlane {
    fn velocity(&self) -> [f64; 3] {
        let tas = true_airspeed_for(self.ias_mps, BASE_ALT_M - self.ned[2]);
        [
            tas * self.course_rad.cos(),
            tas * self.course_rad.sin(),
            -self.climb_mps,
        ]
    }

    fn step(&mut self, intent: Option<&ControlIntent>) {
        if let Some(ControlIntent::Velocity(v)) = intent {
            let (sin, cos) = self.course_rad.sin_cos();
            let (vx, vy) = (f64::from(v.vx), f64::from(v.vy));
            let [north, east] = [vx * cos - vy * sin, vx * sin + vy * cos];
            if north.hypot(east) > 0.1 {
                let err = east.atan2(north) - self.course_rad;
                let err = err.sin().atan2(err.cos());
                let tas = true_airspeed_for(self.ias_mps, BASE_ALT_M - self.ned[2]);
                let max_rate = GRAVITY_MPS2 * BANK_LIMIT_RAD.tan() / tas;
                let rate = (err / 2.0).clamp(-max_rate, max_rate);
                self.course_rad += rate * STEP_S;
            }
            self.climb_mps = -f64::from(v.vz);
        }
        let vel = self.velocity();
        for (pos, v) in self.ned.iter_mut().zip(vel) {
            *pos += v * STEP_S;
        }
    }
}

#[test]
fn magnetic_courses_require_epoch_and_resolve_wmm2025_declination() {
    let plan = WaypointPlan::new(vec![
        wp(0.0, 0.0, BASE_ALT_M, "IF0").with_path(PlannedPath::InitialFix),
        wp(0.0, 1_500.0, BASE_ALT_M, "CF1").with_path(PlannedPath::CourseToFix {
            course_rad: FRAC_PI_2,
            reference: PlannedCourseReference::Magnetic,
        }),
    ]);
    let refused = MissionEngine::from_waypoints(&plan, fixed_wing_config(false));
    assert!(matches!(refused, Err(MissionBuildError::PlanActivation(_))));

    let accepted = MissionEngine::from_waypoints(&plan, fixed_wing_config(true));
    assert!(accepted.is_ok(), "WMM2025 resolves magnetic course cleanly");
}

fn cifp_procedure_plan() -> WaypointPlan {
    let east_hold = PlannedHold {
        inbound_course_rad: FRAC_PI_2,
        reference: PlannedCourseReference::True,
        turn: WaypointTurnDirection::Right,
        length: PlannedHoldLength::DistanceM(900.0),
    };
    let mut plan = WaypointPlan::new(vec![
        wp(0.0, 0.0, BASE_ALT_M, "IF0").with_path(PlannedPath::InitialFix),
        wp(0.0, 1_500.0, BASE_ALT_M, "CF1").with_path(PlannedPath::CourseToFix {
            course_rad: 101.5_f64.to_radians(),
            reference: PlannedCourseReference::Magnetic,
        }),
        wp(0.0, 3_000.0, BASE_ALT_M + 80.0, "CA1")
            .with_altitude(PlannedAltitude::AtOrAbove(BASE_ALT_M + 80.0))
            .with_path(PlannedPath::CourseToAltitude {
                course_rad: 101.5_f64.to_radians(),
                reference: PlannedCourseReference::Magnetic,
            }),
        wp(0.0, 4_800.0, BASE_ALT_M + 80.0, "DF1").with_path(PlannedPath::DirectToFix),
        wp(0.0, 6_600.0, BASE_ALT_M + 160.0, "HA1")
            .with_altitude(PlannedAltitude::AtOrAbove(BASE_ALT_M + 160.0))
            .with_path(PlannedPath::HoldToAltitude(east_hold)),
        wp(0.0, 8_600.0, BASE_ALT_M + 160.0, "HF1").with_path(PlannedPath::HoldToFix(east_hold)),
        wp(0.0, 10_600.0, BASE_ALT_M + 160.0, "HM1").with_path(PlannedPath::HoldManual(east_hold)),
        wp(0.0, 12_200.0, BASE_ALT_M + 160.0, "END1"),
    ]);
    plan.capture_radius_m = Some(90.0);
    plan
}

#[test]
fn cifp_cf_ca_df_and_ha_hf_hm_holds_sequence_in_closed_loop() {
    let plan = cifp_procedure_plan();
    let (mut engine, _) =
        MissionEngine::from_waypoints(&plan, fixed_wing_config(true)).expect("builds");
    let mut plane = SimPlane {
        ned: [0.0, -200.0, 0.0],
        course_rad: FRAC_PI_2,
        ias_mps: IAS_MPS,
        climb_mps: 0.0,
    };

    let mut advances: Vec<(usize, SequenceReason)> = Vec::new();
    let mut hm_exit_sent = false;
    let mut now_ns = 1_000_000_000_u64;

    for step in 0..16_000_u32 {
        now_ns = now_ns.wrapping_add(STEP_NANOS);
        let now = MonotonicNanos::from_nanos(now_ns);
        let sample = OwnshipSample {
            ned: plane.ned,
            ned_velocity: plane.velocity(),
            yaw_rad: Some(plane.course_rad),
            role: TruthRole::SimulationTruth,
            acquired_at: now,
            sequence: step,
        };
        engine.on_ownship(&sample, now);
        let out = engine.tick(now);
        if let Some(act) = out.action
            && act.action == ControlAction::Arm
        {
            engine.on_action_result(act.action_id, true);
        }
        for ev in &out.events {
            if let MissionEvent::LegAdvanced { to_index, reason } = ev {
                advances.push((*to_index, *reason));
            }
        }
        // On HM1 (leg index 6), wait until 1 full circuit completes, then arm exit!
        if !hm_exit_sent
            && engine.nav_guidance().is_some_and(|g| g.leg_index == 6)
            && engine
                .hold_status()
                .is_some_and(|h| h.circuits_completed >= 1 && h.stage == HoldStage::Outbound)
        {
            assert!(engine.exit_hold(), "exit_hold arms manual hold exit");
            hm_exit_sent = true;
        }
        plane.step(out.intent.as_ref());
        if out
            .events
            .iter()
            .any(|e| matches!(e, MissionEvent::LastWaypointSequenced { .. }))
        {
            break;
        }
    }

    assert!(
        hm_exit_sent,
        "HM hold completed >=1 circuit before manual exit: {advances:?}"
    );
    assert_cifp_sequence_reasons(&advances);
}

fn assert_cifp_sequence_reasons(reasons: &[(usize, SequenceReason)]) {
    let indices: Vec<usize> = reasons.iter().map(|(i, _)| *i).collect();
    assert_eq!(
        indices,
        [1, 2, 3, 4, 5, 6, 7],
        "all legs sequenced: {reasons:?}"
    );
    assert_eq!(reasons[2].1, SequenceReason::AltitudeReached, "CA leg");
    assert_eq!(reasons[4].1, SequenceReason::AltitudeReached, "HA hold");
    assert_eq!(reasons[5].1, SequenceReason::HoldCircuitComplete, "HF hold");
    assert_eq!(reasons[6].1, SequenceReason::HoldExited, "HM hold");
}

#[test]
fn mandatory_turn_direction_forces_the_commanded_arc_on_course_reversal() {
    // Aircraft starts heading north (0 rad). Fix REV is south-west (-150 deg),
    // so the shorter turn would be LEFT (-150 deg), but WaypointTurnDirection::Right
    // mandates a 210 deg clockwise RIGHT turn through east (+90 deg).
    let mut plan = WaypointPlan::new(vec![
        wp(0.0, 0.0, BASE_ALT_M, "START").with_path(PlannedPath::InitialFix),
        wp(-2_500.0, -1_400.0, BASE_ALT_M, "REV")
            .with_path(PlannedPath::DirectToFix)
            .with_turn_direction(WaypointTurnDirection::Right),
    ]);
    plan.capture_radius_m = Some(90.0);
    let (mut engine, _) =
        MissionEngine::from_waypoints(&plan, fixed_wing_config(false)).expect("builds");
    let mut plane = SimPlane {
        ned: [0.0, 0.0, 0.0],
        course_rad: 0.0,
        ias_mps: IAS_MPS,
        climb_mps: 0.0,
    };

    let mut passed_east = false;
    let mut went_west_early = false;
    let mut now_ns = 1_000_000_000_u64;
    for step in 0..400_u32 {
        now_ns = now_ns.wrapping_add(STEP_NANOS);
        let now = MonotonicNanos::from_nanos(now_ns);
        let sample = OwnshipSample {
            ned: plane.ned,
            ned_velocity: plane.velocity(),
            yaw_rad: Some(plane.course_rad),
            role: TruthRole::SimulationTruth,
            acquired_at: now,
            sequence: step,
        };
        engine.on_ownship(&sample, now);
        let out = engine.tick(now);
        if let Some(act) = out.action
            && act.action == ControlAction::Arm
        {
            engine.on_action_result(act.action_id, true);
        }
        plane.step(out.intent.as_ref());
        let hdg = plane.course_rad.sin().atan2(plane.course_rad.cos());
        if !passed_east && hdg < -0.15 {
            went_west_early = true;
        }
        if (hdg - FRAC_PI_2).abs() < 0.25 {
            passed_east = true;
        }
        if (hdg + 5.0 * PI / 6.0).abs() < 0.2 {
            break;
        }
    }
    assert!(
        passed_east && !went_west_early,
        "mandatory Right turn turned clockwise through east without turning left first"
    );
}
