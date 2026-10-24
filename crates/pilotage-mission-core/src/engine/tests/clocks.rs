//! The clock contract: a phase reads the simulator's clock only when its deadline or a
//! condition names it, so a task with no navigation data and no simulator runs on the
//! mission clock alone.

use crate::{
    DeadlineClass, EngineEvent, EngineInputError, EngineStart, ExecutionPolicy, ExecutionTarget,
    FlightAction, MissionAction, MissionCapability, MissionCondition, MissionDocument,
    MissionEngine, MissionObservation, MissionPhase, MissionTerminal, PhaseClock, PhaseDeadline,
    TickInput, WallDeadline,
};

use super::support::{RECEIPT_TIMEOUT_NS, WALL_DEADLINE_NS, succeeded};

/// A real vehicle's one-phase mission with no navigation data, `deadline` on its phase.
fn mission(deadline: Option<PhaseDeadline>, capabilities: Vec<MissionCapability>) -> MissionEngine {
    let document = MissionDocument::new(
        "clock-mission".to_owned(),
        None,
        ExecutionPolicy {
            target: ExecutionTarget::RealVehicle,
            retry_limit: 0,
            receipt_timeout_ns: RECEIPT_TIMEOUT_NS,
        },
        vec![MissionPhase {
            id: "arm".to_owned(),
            required_capabilities: capabilities,
            entry_conditions: Vec::new(),
            action: MissionAction::Flight(FlightAction::Arm {}),
            cleanup_actions: Vec::new(),
            completion_conditions: vec![MissionCondition::Always {}],
            abort_conditions: Vec::new(),
            depends_on: Vec::new(),
            deadline,
        }],
    )
    .expect("a valid document");
    let digest = document.identity.content_digest;
    MissionEngine::start(
        document,
        EngineStart {
            host_target: ExecutionTarget::RealVehicle,
            mission_time_ns: 0,
            simulator_time_ns: None,
            wall_time_ns: 0,
            wall_deadline: WallDeadline {
                mission_content_digest: digest,
                expires_at_ns: WALL_DEADLINE_NS,
            },
        },
    )
    .expect("starts without a simulator clock")
}

fn at(mission_time_ns: u64, receipts: Vec<crate::DirectiveReceipt>) -> TickInput {
    TickInput {
        mission_time_ns,
        simulator_time_ns: None,
        wall_time_ns: mission_time_ns,
        observation: MissionObservation::default(),
        receipts,
    }
}

#[test]
fn a_task_with_no_navigation_data_and_no_simulator_runs_on_the_mission_clock() {
    let mut engine = mission(None, vec![MissionCapability::ArmDisarm]);
    let first = engine
        .tick(at(0, Vec::new()))
        .expect("no simulator clock is needed");
    assert_eq!(first.directives.len(), 1);
    let done = engine
        .tick(at(1, vec![succeeded(&first)]))
        .expect("completes");
    assert!(matches!(
        done.state,
        crate::EngineState::Terminal {
            result: MissionTerminal::Complete { .. }
        }
    ));
}

#[test]
fn a_mission_clock_deadline_ends_a_phase_without_a_simulator() {
    let deadline = Some(PhaseDeadline {
        clock: PhaseClock::Mission,
        duration_ns: 10,
    });
    let mut engine = mission(deadline, vec![MissionCapability::ArmDisarm]);
    engine.tick(at(0, Vec::new())).expect("enters");
    let late = engine.tick(at(10, Vec::new())).expect("ticks");
    assert!(late.events.iter().any(|event| matches!(
        event,
        EngineEvent::DeadlineExceeded {
            deadline: DeadlineClass::Phase {
                clock: PhaseClock::Mission,
                ..
            }
        }
    )));
}

#[test]
fn a_phase_on_the_simulator_clock_refuses_input_without_one() {
    let deadline = Some(PhaseDeadline {
        clock: PhaseClock::Simulator,
        duration_ns: 10,
    });
    let mut engine = mission(
        deadline,
        vec![
            MissionCapability::ArmDisarm,
            MissionCapability::SimulatorTime,
        ],
    );
    assert_eq!(
        engine.tick(at(0, Vec::new())).unwrap_err(),
        EngineInputError::SimulatorTimeMissing {
            phase_id: "arm".to_owned()
        }
    );
}
