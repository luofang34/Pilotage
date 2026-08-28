use crate::{
    Digest, DirectiveReceipt, EngineStart, EngineState, ExecutionPolicy, ExecutionTarget,
    FlightAction, MissionAction, MissionCapability, MissionCondition, MissionDocument,
    MissionEngine, MissionObservation, MissionPhase, MissionTerminal, NavigationDataIdentity,
    ReceiptResult, TickInput, TickOutput, WallDeadline,
};

pub(super) const PHASE_DEADLINE_NS: u64 = 100;
pub(super) const RECEIPT_TIMEOUT_NS: u64 = 20;
pub(super) const WALL_DEADLINE_NS: u64 = 1_000;

pub(super) fn digest(byte: u8) -> Digest {
    Digest::from_bytes([byte; 32])
}

pub(super) fn phase(id: &str) -> MissionPhase {
    MissionPhase {
        id: id.to_owned(),
        required_capabilities: vec![
            MissionCapability::SimulatorTime,
            MissionCapability::ArmDisarm,
            MissionCapability::ContactState,
        ],
        entry_conditions: Vec::new(),
        action: MissionAction::Flight(FlightAction::Arm {}),
        cleanup_actions: Vec::new(),
        completion_conditions: Vec::new(),
        abort_conditions: Vec::new(),
        simulator_time_deadline_ns: PHASE_DEADLINE_NS,
    }
}

pub(super) fn document(phases: Vec<MissionPhase>) -> MissionDocument {
    MissionDocument::new(
        "engine-mission-1".to_owned(),
        NavigationDataIdentity {
            cycle: "2608".to_owned(),
            snapshot_id: "nav-engine-1".to_owned(),
            snapshot_digest: digest(7),
        },
        ExecutionPolicy {
            target: ExecutionTarget::Simulator,
            retry_limit: 1,
            receipt_timeout_ns: RECEIPT_TIMEOUT_NS,
        },
        phases,
    )
    .expect("test mission")
}

pub(super) fn engine(document: MissionDocument) -> MissionEngine {
    let mission_content_digest = document.identity.content_digest;
    MissionEngine::start(
        document,
        EngineStart {
            host_target: ExecutionTarget::Simulator,
            simulator_time_ns: 0,
            wall_time_ns: 0,
            wall_deadline: WallDeadline {
                mission_content_digest,
                expires_at_ns: WALL_DEADLINE_NS,
            },
        },
    )
    .expect("start test mission")
}

pub(super) fn tick(
    engine: &mut MissionEngine,
    simulator_time_ns: u64,
    wall_time_ns: u64,
    observation: MissionObservation,
    receipts: Vec<DirectiveReceipt>,
) -> TickOutput {
    engine
        .tick(TickInput {
            simulator_time_ns,
            wall_time_ns,
            observation,
            receipts,
        })
        .expect("valid tick")
}

pub(super) fn succeeded(output: &TickOutput) -> DirectiveReceipt {
    DirectiveReceipt {
        action_id: output.directives[0].context().action_id,
        result: ReceiptResult::Succeeded {},
    }
}

pub(super) fn terminal(output: &TickOutput) -> &MissionTerminal {
    let EngineState::Terminal { result } = &output.state else {
        panic!("expected terminal output: {output:?}");
    };
    result
}

pub(super) fn condition_phase(
    entry_conditions: Vec<MissionCondition>,
    completion_conditions: Vec<MissionCondition>,
    abort_conditions: Vec<MissionCondition>,
) -> MissionPhase {
    MissionPhase {
        entry_conditions,
        completion_conditions,
        abort_conditions,
        ..phase("condition-phase")
    }
}
