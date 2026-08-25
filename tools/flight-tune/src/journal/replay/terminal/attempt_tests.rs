use crate::journal::OperationStatus;
use crate::journal::replay::{attempt, terminal};
use crate::score::aggregate_runs;
use crate::{CandidateEvaluation, RunRecord, ScenarioSet};

use super::{ReplayFixture, RunArtifacts, SemanticCase, hard_gate_failure, run_record};

#[test]
fn passing_attempt_requires_the_exact_ordered_committed_runs() {
    let (mut fixture, artifacts, runs) = committed_passing_attempt();
    let aggregate = aggregate_runs(&runs, ScenarioSet::Training).expect("aggregate runs");

    attempt::complete(
        &mut fixture.state,
        0,
        &CandidateEvaluation::Passed { aggregate, runs },
        Some(true),
        &fixture.stage,
        fixture.session.fixed_seed,
    )
    .expect("complete exact attempt");

    assert!(fixture.state.pending.as_ref().is_some_and(|pending| {
        pending
            .outcome
            .as_ref()
            .is_some_and(|outcome| matches!(outcome.evaluation, CandidateEvaluation::Passed { .. }))
    }));
    assert_eq!(artifacts.len(), 2);
}

#[test]
fn passing_attempt_rejects_a_different_valid_run_record() {
    let (mut fixture, _artifacts, mut runs) = committed_passing_attempt();
    runs[1].loss = 0.8;
    let aggregate = aggregate_runs(&runs, ScenarioSet::Training).expect("aggregate changed runs");
    let evaluation = CandidateEvaluation::Passed { aggregate, runs };

    assert!(
        attempt::complete(
            &mut fixture.state,
            0,
            &evaluation,
            Some(true),
            &fixture.stage,
            fixture.session.fixed_seed,
        )
        .is_err()
    );
}

#[test]
fn attempt_cannot_complete_with_an_uncommitted_run() {
    let mut fixture = ReplayFixture::new();
    let first = fixture.prepare_run(0, SemanticCase::ScenarioComplete);
    fixture.commit(&first);
    let second = fixture.prepare_run(1, SemanticCase::ScenarioComplete);
    let runs = vec![
        run_record(first.intent.context(), 0),
        run_record(second.intent.context(), 1),
    ];
    let aggregate = aggregate_runs(&runs, ScenarioSet::Training).expect("aggregate runs");
    let evaluation = CandidateEvaluation::Passed { aggregate, runs };

    assert!(
        attempt::complete(
            &mut fixture.state,
            0,
            &evaluation,
            Some(true),
            &fixture.stage,
            fixture.session.fixed_seed,
        )
        .is_err()
    );
}

#[test]
fn hard_gate_attempt_requires_the_exact_final_abort() {
    let mut fixture = ReplayFixture::new();
    let artifacts = fixture.prepare_run(0, SemanticCase::HardGateAbort);
    fixture.commit(&artifacts);
    let failure = hard_gate_failure(artifacts.intent.context());
    let exact = CandidateEvaluation::HardGateFailed {
        failure: failure.clone(),
        completed_runs: Vec::new(),
    };

    attempt::complete(
        &mut fixture.state,
        0,
        &exact,
        Some(false),
        &fixture.stage,
        fixture.session.fixed_seed,
    )
    .expect("complete hard gate attempt");

    let mut different = ReplayFixture::new();
    let artifacts = different.prepare_run(0, SemanticCase::HardGateAbort);
    different.commit(&artifacts);
    let mut changed = failure;
    changed.elapsed_ms = changed.elapsed_ms.wrapping_add(1);
    let evaluation = CandidateEvaluation::HardGateFailed {
        failure: changed,
        completed_runs: Vec::new(),
    };
    assert!(
        attempt::complete(
            &mut different.state,
            0,
            &evaluation,
            Some(false),
            &different.stage,
            different.session.fixed_seed,
        )
        .is_err()
    );
}

#[test]
fn attempt_completed_rejects_a_quarantine_receipt() {
    let mut fixture = ReplayFixture::new();
    let artifacts = fixture.prepare_run(0, SemanticCase::ExecutionError);
    fixture.commit(&artifacts);
    let evaluation = CandidateEvaluation::Quarantined {
        reason: terminal::quarantine_reason(&artifacts.receipt).expect("derive reason"),
    };

    assert!(
        attempt::complete(
            &mut fixture.state,
            0,
            &evaluation,
            Some(false),
            &fixture.stage,
            fixture.session.fixed_seed,
        )
        .is_err()
    );
}

#[test]
fn attempt_quarantine_requires_the_one_final_receipt_and_reason() {
    let mut fixture = ReplayFixture::new();
    let first = fixture.prepare_run(0, SemanticCase::ScenarioComplete);
    fixture.commit(&first);
    let final_run = fixture.prepare_run(1, SemanticCase::ExecutionError);
    fixture.commit(&final_run);
    let reason = terminal::quarantine_reason(&final_run.receipt).expect("derive reason");

    assert!(attempt::quarantine(&mut fixture.state, 0, "different reason").is_err());
    attempt::quarantine(&mut fixture.state, 0, &reason).expect("quarantine exact attempt");
}

#[test]
fn cleanup_failure_keeps_the_closed_attempt_pending() {
    let (mut fixture, _artifacts, runs) = committed_passing_attempt();
    let aggregate = aggregate_runs(&runs, ScenarioSet::Training).expect("aggregate runs");
    attempt::complete(
        &mut fixture.state,
        0,
        &CandidateEvaluation::Passed { aggregate, runs },
        Some(true),
        &fixture.stage,
        fixture.session.fixed_seed,
    )
    .expect("complete attempt");

    attempt::cleanup(
        &mut fixture.state,
        0,
        &OperationStatus::Failed {
            detail: "simulator cleanup failed".to_owned(),
        },
    )
    .expect("record failed cleanup");
    assert!(fixture.state.pending.is_some());
    assert!(attempt::cleanup(&mut fixture.state, 0, &OperationStatus::NotRequired).is_err());
    attempt::cleanup(&mut fixture.state, 0, &OperationStatus::Succeeded)
        .expect("record successful cleanup");
    assert!(fixture.state.pending.is_none());
}

#[test]
fn cleanup_before_attempt_closure_is_rejected() {
    let mut fixture = ReplayFixture::new();

    assert!(attempt::cleanup(&mut fixture.state, 0, &OperationStatus::Succeeded).is_err());
}

fn committed_passing_attempt() -> (ReplayFixture, Vec<RunArtifacts>, Vec<RunRecord>) {
    let mut fixture = ReplayFixture::new();
    let first = fixture.prepare_run(0, SemanticCase::ScenarioComplete);
    fixture.commit(&first);
    let second = fixture.prepare_run(1, SemanticCase::ScenarioComplete);
    fixture.commit(&second);
    let runs = vec![
        run_record(first.intent.context(), 0),
        run_record(second.intent.context(), 1),
    ];
    (fixture, vec![first, second], runs)
}
