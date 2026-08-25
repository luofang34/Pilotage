use crate::{
    ArtifactIdentity, JournalEvent, RunBindingReceipt, RunTerminalClass, RunTerminalDiagnostic,
    RunTerminalIntent, RunTerminalReceipt, RunTerminalRecoveryState, RunTerminalReport,
    RunTerminalSemanticOutcome,
};

use super::{
    ReplayFixture, SemanticCase, apply_event, fixed_digest, successful_outcomes, terminal_events,
};

#[test]
fn run_commit_requires_the_exact_saved_binding() {
    let mut fixture = ReplayFixture::new();
    let artifacts = fixture.prepare_run(0, SemanticCase::ScenarioComplete);
    let mut events = terminal_events(&artifacts);
    for event in events.drain(..3) {
        apply_event(&mut fixture.state, &event, &fixture.session).expect("save base chain");
    }
    let foreign =
        ArtifactIdentity::new("foreign-vehicle", fixed_digest(78)).expect("create foreign adapter");
    let binding = RunBindingReceipt::new(artifacts.intent.context(), &artifacts.plan, foreign)
        .expect("create foreign binding");
    let receipt = RunTerminalReceipt::new(
        &binding,
        &artifacts.intent,
        &artifacts.report,
        artifacts.base_class,
        fixed_digest(94),
    )
    .expect("create foreign receipt");
    let event = JournalEvent::RunCommitted {
        trial_id: 0,
        run_index: 0,
        receipt: Box::new(receipt),
    };

    assert!(apply_event(&mut fixture.state, &event, &fixture.session).is_err());
}

#[test]
fn evidence_failure_rejects_the_base_completed_receipt() {
    let mut fixture = ReplayFixture::new();
    let artifacts = fixture.prepare_run(0, SemanticCase::ScenarioComplete);
    let mut events = terminal_events(&artifacts);
    for event in events.drain(..3) {
        apply_event(&mut fixture.state, &event, &fixture.session).expect("save base chain");
    }
    let class = RunTerminalClass::evidence_failure(&artifacts.intent, &artifacts.report)
        .expect("create evidence failure");
    apply_event(
        &mut fixture.state,
        &JournalEvent::RunTerminalEvidenceFailureRecorded {
            trial_id: 0,
            run_index: 0,
            class,
        },
        &fixture.session,
    )
    .expect("save evidence failure");
    let commit = JournalEvent::RunCommitted {
        trial_id: 0,
        run_index: 0,
        receipt: Box::new(artifacts.receipt),
    };

    assert!(apply_event(&mut fixture.state, &commit, &fixture.session).is_err());
}

#[test]
fn duplicate_run_commit_is_rejected() {
    let mut fixture = ReplayFixture::new();
    let artifacts = fixture.prepare_run(0, SemanticCase::ScenarioComplete);
    fixture.commit(&artifacts);
    let event = JournalEvent::RunCommitted {
        trial_id: 0,
        run_index: 0,
        receipt: Box::new(artifacts.receipt),
    };

    assert!(apply_event(&mut fixture.state, &event, &fixture.session).is_err());
}

#[test]
fn run_commit_rejects_a_coherent_diagnostic_rechain() {
    let mut fixture = ReplayFixture::new();
    let artifacts = fixture.prepare_run(0, SemanticCase::ExecutionError);
    let mut events = terminal_events(&artifacts);
    for event in events.drain(..3) {
        apply_event(&mut fixture.state, &event, &fixture.session).expect("save exact chain");
    }
    let changed_intent = RunTerminalIntent::new(
        artifacts.intent.context(),
        artifacts.intent.run_intent_digest(),
        RunTerminalSemanticOutcome::ExecutionError {
            diagnostic: RunTerminalDiagnostic::new("the simulator connection ended")
                .expect("create changed diagnostic"),
        },
    )
    .expect("create changed intent");
    let changed_report = RunTerminalReport::new(
        &artifacts.plan,
        &changed_intent,
        RunTerminalRecoveryState::Live,
        successful_outcomes(),
    )
    .expect("create changed report");
    let changed_class = RunTerminalClass::classify(&changed_intent, &changed_report)
        .expect("classify changed report");
    let changed_receipt = RunTerminalReceipt::new(
        &artifacts.binding,
        &changed_intent,
        &changed_report,
        changed_class,
        fixed_digest(95),
    )
    .expect("create changed receipt");
    let event = JournalEvent::RunCommitted {
        trial_id: 0,
        run_index: 0,
        receipt: Box::new(changed_receipt),
    };

    assert!(apply_event(&mut fixture.state, &event, &fixture.session).is_err());
}
