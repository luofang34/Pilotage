#![cfg(test)]

//! The steering source a display reads under simulation truth: the
//! engine fuses the truth as a synthesized satellite-class fix, and the
//! display must not be led to name a satellite receiver for it.

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use pilotage_mission::{
    ExecutionTarget, FlightPhase, MissionConfig, MissionEngine, OwnshipSample, OwnshipSource,
    PlannedWaypoint, PositionSource, StartWithheldReason, SteeringIntegrity, SteeringSource,
    TruthRole, WaypointPlan,
};

const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 50_000_000;

fn engine() -> MissionEngine {
    let anchor = GeodeticPosition::new(47.0_f64.to_radians(), 8.0_f64.to_radians(), 500.0);
    let mut config = MissionConfig::new(String::new(), anchor, CLOCK);
    config.ownship_source = OwnshipSource::SimulationTruth;
    config.execution_target = ExecutionTarget::Simulator;
    let plan = WaypointPlan::new(vec![
        PlannedWaypoint::new(47.001, 8.0, 520.0),
        PlannedWaypoint::new(47.002, 8.0, 520.0),
    ]);
    MissionEngine::from_waypoints(&plan, config)
        .expect("builds")
        .0
}

#[test]
fn simulation_truth_is_named_as_such_with_no_sensor_class() {
    let mut engine = engine();
    assert_eq!(
        engine.navigation_display().steering,
        SteeringSource::new(
            PositionSource::SimulationTruth,
            None,
            SteeringIntegrity::NoSolution {
                reason: StartWithheldReason::NavigationNotStarted
            },
        )
    );
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut published = false;
    for sequence in 0..200_u32 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        let sample = OwnshipSample {
            ned: [0.0; 3],
            ned_velocity: [0.0; 3],
            yaw_rad: Some(0.0),
            role: TruthRole::SimulationTruth,
            acquired_at: now,
            sequence,
        };
        engine.on_ownship(&sample, now);
        engine.tick(now);
        if engine.solution().is_some() {
            published = true;
            break;
        }
    }
    assert!(published, "the fused truth publishes a solution");
    let display = engine.navigation_display();
    assert_eq!(display.phase, FlightPhase::Enroute);
    assert_eq!(display.steering.source, PositionSource::SimulationTruth);
    assert_eq!(
        display.steering.contributing, None,
        "the synthesized fix's class is not reported as a sensor"
    );
    assert!(
        matches!(
            display.steering.integrity,
            SteeringIntegrity::WithinLimits { .. }
        ),
        "{:?}",
        display.steering
    );
}
