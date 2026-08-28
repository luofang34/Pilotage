use crate::{
    ActionId, DirectiveReceipt, EngineInputError, EngineStart, EngineStartError, EngineState,
    ExecutionTarget, MissionDocument, MissionEngine, MissionObservation, ReceiptResult, TickInput,
    WallDeadline,
};

use super::support::{WALL_DEADLINE_NS, digest, document, engine, phase, succeeded, tick};

#[test]
fn identical_document_bytes_and_inputs_produce_identical_outputs() {
    let bytes = document(vec![phase("only")])
        .to_canonical_json()
        .expect("canonical mission");
    let first = run_bytes(&bytes);
    let second = run_bytes(&bytes);
    assert_eq!(first, second);
    assert!(matches!(
        first[1].state,
        EngineState::Terminal {
            result: crate::MissionTerminal::Complete { .. }
        }
    ));
}

fn run_bytes(bytes: &[u8]) -> Vec<crate::TickOutput> {
    let document = MissionDocument::from_json(bytes).expect("decode mission");
    let mut engine = engine(document);
    let first = tick(&mut engine, 0, 0, MissionObservation::default(), Vec::new());
    let second = tick(
        &mut engine,
        1,
        1,
        MissionObservation::default(),
        vec![succeeded(&first)],
    );
    vec![first, second]
}

#[test]
fn stale_receipt_is_rejected_without_changing_state() {
    let mut engine = engine(document(vec![phase("only")]));
    let first = tick(&mut engine, 0, 0, MissionObservation::default(), Vec::new());
    let expected = engine.state();
    let error = engine
        .tick(TickInput {
            simulator_time_ns: 1,
            wall_time_ns: 1,
            observation: MissionObservation::default(),
            receipts: vec![DirectiveReceipt {
                action_id: ActionId(99),
                result: ReceiptResult::Succeeded {},
            }],
        })
        .expect_err("stale receipt");
    assert!(matches!(error, EngineInputError::StaleReceipt { .. }));
    assert_eq!(engine.state(), expected);
    let correct = tick(
        &mut engine,
        1,
        1,
        MissionObservation::default(),
        vec![succeeded(&first)],
    );
    assert!(matches!(correct.state, EngineState::Terminal { .. }));
}

#[test]
fn action_identifier_wrap_skips_zero_and_replaces_the_resolved_attempt() {
    let mut engine = engine(document(vec![phase("wrap")]));
    engine.last_action_id = u32::MAX.wrapping_sub(1);
    let first = tick(&mut engine, 0, 0, MissionObservation::default(), Vec::new());
    let first_id = first.directives[0].context().action_id;
    assert_eq!(first_id.get(), u32::MAX);
    let retry = tick(
        &mut engine,
        1,
        1,
        MissionObservation::default(),
        vec![DirectiveReceipt {
            action_id: first_id,
            result: ReceiptResult::Retryable {
                detail: "try again".to_owned(),
            },
        }],
    );
    let retry_id = retry.directives[0].context().action_id;
    assert_eq!(retry_id.get(), 1);
    assert_ne!(retry_id, first_id);
    assert!(matches!(
        retry.state,
        EngineState::Running {
            stage: crate::PhaseStage::WaitingForReceipt { action_id },
            ..
        } if action_id == retry_id
    ));
    let stale = engine
        .tick(TickInput {
            simulator_time_ns: 2,
            wall_time_ns: 2,
            observation: MissionObservation::default(),
            receipts: vec![DirectiveReceipt {
                action_id: first_id,
                result: ReceiptResult::Succeeded {},
            }],
        })
        .expect_err("resolved identifier must be stale");
    assert!(matches!(stale, EngineInputError::StaleReceipt { .. }));
}

#[test]
fn wall_deadline_identity_must_match_the_mission_digest() {
    let document = document(vec![phase("identity")]);
    let error = MissionEngine::start(
        document,
        EngineStart {
            host_target: ExecutionTarget::Simulator,
            simulator_time_ns: 0,
            wall_time_ns: 0,
            wall_deadline: WallDeadline {
                mission_content_digest: digest(99),
                expires_at_ns: WALL_DEADLINE_NS,
            },
        },
    )
    .expect_err("deadline identity mismatch");
    assert!(matches!(
        error,
        EngineStartError::WallDeadlineIdentity { .. }
    ));
}
