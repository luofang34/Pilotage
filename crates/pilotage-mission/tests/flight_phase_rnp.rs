#![cfg(test)]

//! Closed-loop verification of flight phase (`ENR` / `TERM` / `APR` / `MISSED`)
//! and RNP (`2.0` / `1.0` / `0.3 NM` and procedure-specified RNP tightening)
//! across SID, en-route, CIFP approach, and missed-approach legs.

use core::f64::consts::FRAC_PI_2;

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use navigate_geodesy::NedOffset;
use navigate_geodesy::wgs84::tangent_plane;
use pilotage_mission::{
    APPROACH_RNP_NM, CifpProcedureKind, ENROUTE_RNP_NM, ExecutionTarget, FlightPhase, LegRole,
    M_PER_NM, MissionConfig, MissionEngine, MissionEvent, OwnshipSample, OwnshipSource,
    PlannedCourseReference, PlannedPath, PlannedWaypoint, ProcedureFixRole, TERMINAL_RNP_NM,
    TruthRole, WaypointPlan, WaypointTurn,
};
use pilotage_protocol::{ControlAction, ControlIntent};

const CLOCK: ClockDomainId = ClockDomainId::new(11);
const STEP_NANOS: u64 = 100_000_000;
const STEP_S: f64 = 0.1;
const ALT_M: f64 = 600.0;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(39.0_f64.to_radians(), -75.5_f64.to_radians(), ALT_M)
}

fn wp_at(east_m: f64, name: &str, role: LegRole) -> PlannedWaypoint {
    let plane = tangent_plane(anchor()).expect("anchor");
    let pos = plane.from_ned(&NedOffset::new(0.0, east_m, 0.0));
    PlannedWaypoint::new(
        pos.latitude_rad.to_degrees(),
        pos.longitude_rad.to_degrees(),
        ALT_M,
    )
    .with_name(name)
    .with_turn(WaypointTurn::FlyOver)
    .with_role(role)
}

fn cifp_route_plan() -> WaypointPlan {
    let sid_role = LegRole::from_cifp(CifpProcedureKind::Sid, None, false);
    let iaf_role = LegRole::from_cifp(
        CifpProcedureKind::Approach,
        Some(ProcedureFixRole::InitialApproachFix),
        false,
    );
    let faf_role = LegRole::from_cifp(
        CifpProcedureKind::Approach,
        Some(ProcedureFixRole::FinalApproachFix),
        false,
    );
    let mapt_role = LegRole::from_cifp(
        CifpProcedureKind::Approach,
        Some(ProcedureFixRole::MissedApproachPoint),
        false,
    );
    let mahf_role = LegRole::from_cifp(
        CifpProcedureKind::Approach,
        Some(ProcedureFixRole::MissedApproachHoldingFix),
        true,
    );

    let mut plan = WaypointPlan::new(vec![
        wp_at(0.0, "RW09", sid_role).with_path(PlannedPath::InitialFix),
        wp_at(80.0, "SID1", sid_role),
        wp_at(160.0, "CRZ1", LegRole::Enroute),
        wp_at(240.0, "IAF1", iaf_role),
        wp_at(320.0, "FAF1", faf_role).with_path(PlannedPath::CourseToFix {
            course_rad: FRAC_PI_2,
            reference: PlannedCourseReference::True,
        }),
        wp_at(400.0, "MAPT", mapt_role)
            .with_path(PlannedPath::CourseToFix {
                course_rad: FRAC_PI_2,
                reference: PlannedCourseReference::True,
            })
            .with_rnp_nm(0.15),
        wp_at(480.0, "MAHF", mahf_role),
    ]);
    plan.capture_radius_m = Some(12.0);
    plan.arrival_radius_m = 6.0;
    plan
}

fn build_engine(plan: &WaypointPlan) -> MissionEngine {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = OwnshipSource::SimulationTruth;
    config.execution_target = ExecutionTarget::Simulator;
    config.cruise_height_m = 0.0;
    config.cruise_mps = 20.0;
    config.limits.max_horizontal_mps = 20.0;
    MissionEngine::from_waypoints(plan, config)
        .expect("plan builds")
        .0
}

fn truth_sample(
    ned: [f64; 3],
    vel: [f64; 3],
    yaw: f64,
    now: MonotonicNanos,
    sequence: u32,
) -> OwnshipSample {
    OwnshipSample {
        ned,
        ned_velocity: vel,
        yaw_rad: Some(yaw),
        role: TruthRole::SimulationTruth,
        acquired_at: now,
        sequence,
    }
}

#[test]
fn cifp_linked_route_switches_phase_and_rnp_across_departure_enroute_arrival_approach_and_missed() {
    let plan = cifp_route_plan();
    let mut engine = build_engine(&plan);

    let mut ned = [0.0_f64; 3];
    let mut vel = [0.0_f64; 3];
    let mut yaw = FRAC_PI_2;
    let mut now_ns: u64 = 1_000_000_000;
    let mut seen_by_leg: Vec<(u32, FlightPhase, f64)> = Vec::new();

    for step in 0..1_500_u32 {
        now_ns = now_ns.wrapping_add(STEP_NANOS);
        let now = MonotonicNanos::from_nanos(now_ns);
        engine.on_ownship(&truth_sample(ned, vel, yaw, now, step), now);
        let out = engine.tick(now);
        if let Some(act) = out.action
            && act.action == ControlAction::Arm
        {
            engine.on_action_result(act.action_id, true);
        }
        if let Some(ControlIntent::Velocity(v)) = out.intent {
            let (sin, cos) = yaw.sin_cos();
            let (vx, vy) = (f64::from(v.vx), f64::from(v.vy));
            vel = [vx * cos - vy * sin, vx * sin + vy * cos, f64::from(v.vz)];
            yaw += f64::from(v.yaw_rate) * STEP_S;
        }
        for axis in 0..3 {
            ned[axis] += vel[axis] * STEP_S;
        }
        if let Some(guidance) = engine.nav_guidance() {
            let display = engine.navigation_display();
            assert_eq!(display.phase, guidance.phase);
            assert!((display.rnp_m - guidance.rnp_m).abs() < 1e-9);
            if seen_by_leg.last().map(|s| s.0) != Some(guidance.leg_index) {
                seen_by_leg.push((guidance.leg_index, guidance.phase, guidance.rnp_m));
            }
        }
        if out
            .events
            .iter()
            .any(|e| matches!(e, MissionEvent::LastWaypointSequenced { .. }))
        {
            break;
        }
    }

    let expected = [
        (1, FlightPhase::Terminal, TERMINAL_RNP_NM * M_PER_NM),
        (2, FlightPhase::Enroute, ENROUTE_RNP_NM * M_PER_NM),
        (3, FlightPhase::Terminal, TERMINAL_RNP_NM * M_PER_NM),
        (4, FlightPhase::Terminal, TERMINAL_RNP_NM * M_PER_NM),
        (5, FlightPhase::Approach, 0.15 * M_PER_NM),
        (6, FlightPhase::Missed, TERMINAL_RNP_NM * M_PER_NM),
    ];
    assert_eq!(seen_by_leg.len(), expected.len(), "{seen_by_leg:?}");
    for ((leg, phase, rnp_m), (exp_leg, exp_phase, exp_rnp_m)) in
        seen_by_leg.iter().zip(expected.iter())
    {
        assert_eq!(leg, exp_leg);
        assert_eq!(phase, exp_phase, "leg {leg}");
        assert!(
            (rnp_m - exp_rnp_m).abs() < 1e-6,
            "leg {leg}: got {rnp_m}, expected {exp_rnp_m}"
        );
    }
}

#[test]
fn approach_default_rnp_is_point_three_nm_and_loose_procedure_rnp_never_widens_it() {
    let mut plan = WaypointPlan::new(vec![
        wp_at(0.0, "FAF", LegRole::Arrival).with_path(PlannedPath::InitialFix),
        wp_at(100.0, "SDF", LegRole::Approach),
        wp_at(200.0, "MAPT", LegRole::Approach).with_rnp_nm(2.5),
    ]);
    plan.capture_radius_m = Some(12.0);
    let mut engine = build_engine(&plan);

    // Arm and enter FollowPlan at the start fix.
    let mut now_ns = 1_000_000_000_u64;
    for seq in 1..=3_u32 {
        now_ns = now_ns.wrapping_add(STEP_NANOS);
        let now = MonotonicNanos::from_nanos(now_ns);
        engine.on_ownship(
            &truth_sample([0.0, 0.0, 0.0], [0.0, 15.0, 0.0], FRAC_PI_2, now, seq),
            now,
        );
        let out = engine.tick(now);
        if let Some(act) = out.action
            && act.action == ControlAction::Arm
        {
            engine.on_action_result(act.action_id, true);
        }
    }
    let guidance = engine.nav_guidance().expect("guidance on first leg");
    assert_eq!(guidance.leg_index, 1);
    assert_eq!(guidance.phase, FlightPhase::Approach);
    assert!((guidance.rnp_m - APPROACH_RNP_NM * M_PER_NM).abs() < 1e-6);

    // Advance past SDF onto MAPT (which tried to specify 2.5 NM > 0.3 NM).
    for seq in 4..=40_u32 {
        now_ns = now_ns.wrapping_add(STEP_NANOS);
        let t2 = MonotonicNanos::from_nanos(now_ns);
        engine.on_ownship(
            &truth_sample([0.0, 100.0, 0.0], [0.0, 15.0, 0.0], FRAC_PI_2, t2, seq),
            t2,
        );
        let _ = engine.tick(t2);
    }
    let g2 = engine.nav_guidance().expect("guidance on second leg");
    assert_eq!(g2.leg_index, 2);
    assert_eq!(g2.phase, FlightPhase::Approach);
    assert!((g2.rnp_m - APPROACH_RNP_NM * M_PER_NM).abs() < 1e-6);
}
