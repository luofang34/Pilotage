//! Attacks on the scores the independent replay derives for itself.
//!
//! The training incumbent decision turns on a mean loss. If the verifier read
//! that number back out of the document it is checking, the document would be
//! its own authority. Each case below changes one stored score and requires
//! the verifier to notice that the run records no longer produce it.

use flight_tune::{CandidateEvaluation, FinalQualificationOutcome, JournalEvent};

use crate::CampaignEvidence;

use super::producer_rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, QuadraticMetric, SequenceStrategy,
    TestDirectory, candidate, stage,
};
use super::{stated_policy, verify};

/// A sealed campaign whose chain carries real training run records.
fn sealed_campaign(name: &str) -> CampaignEvidence {
    let directory = TestDirectory::new(name);
    let state = FakeHandle::new();
    let mut tuner = flight_tune::Tuner::open_or_resume(
        directory.path(),
        stage(),
        91,
        candidate(0.0),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        EnvelopeGates::new(2.0),
        QuadraticMetric::new(state),
        SequenceStrategy::new(vec![0.5]),
    )
    .expect("open a producer tuner");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run producer training");
    tuner.freeze_candidate().expect("freeze producer candidate");
    tuner
        .run_promotion_once_blocking()
        .expect("run producer promotion");
    assert_eq!(
        tuner
            .run_final_qualification_once_blocking()
            .expect("run producer final qualification"),
        FinalQualificationOutcome::Qualified
    );
    let snapshot = tuner
        .journal()
        .verified_evidence_snapshot()
        .expect("read producer evidence snapshot");
    CampaignEvidence::new(snapshot).expect("verify the producer snapshot")
}

/// Relinks a chain a case has changed, so its refusal is not the link check.
fn rechain(evidence: &mut CampaignEvidence) {
    super::retry_attacks::rechain(evidence);
}

fn assert_refused(evidence: &CampaignEvidence, case: &str) {
    let error = verify(evidence)
        .err()
        .unwrap_or_else(|| panic!("the verifier accepted a changed {case}"));
    assert!(
        !format!("{error}").contains("chain changed"),
        "the {case} case was refused before the score was read: {error}"
    );
}

/// One named change to a stored evaluation.
type Case = (&'static str, fn(&mut CandidateEvaluation));

/// Applies one change to the first completed training evaluation in the chain.
fn change_training_evaluation(
    evidence: &mut CampaignEvidence,
    change: impl Fn(&mut CandidateEvaluation),
) {
    let mut applied = false;
    for record in &mut evidence.journal.authority.journal_chain {
        if let JournalEvent::AttemptCompleted {
            evaluation,
            proof: None,
            ..
        } = &mut record.entry.event
            && !applied
        {
            change(evaluation);
            applied = true;
        }
    }
    assert!(applied, "the producer completed a training attempt");
    rechain(evidence);
}

#[test]
fn an_untouched_campaign_qualifies() {
    let evidence = sealed_campaign("aggregate-attack-baseline");
    let required = stated_policy(&evidence);

    verify(&evidence)
        .and_then(|verified| verified.verify_qualified(&required))
        .expect("an untouched campaign qualifies");
}

#[test]
fn every_stored_aggregate_field_is_derived_again() {
    // One case per field, so a verifier that recomputed only the mean would
    // still be caught by the rest.
    let cases: [Case; 6] = [
        ("mean", |evaluation| {
            if let CandidateEvaluation::Passed { aggregate, .. } = evaluation {
                aggregate.mean_loss += 0.25;
            }
        }),
        ("p95", |evaluation| {
            if let CandidateEvaluation::Passed { aggregate, .. } = evaluation {
                aggregate.p95_loss += 0.25;
            }
        }),
        ("variance", |evaluation| {
            if let CandidateEvaluation::Passed { aggregate, .. } = evaluation {
                aggregate.loss_variance += 0.25;
            }
        }),
        ("confidence lower", |evaluation| {
            if let CandidateEvaluation::Passed { aggregate, .. } = evaluation {
                aggregate.loss_confidence_95.lower -= 0.25;
            }
        }),
        ("confidence upper", |evaluation| {
            if let CandidateEvaluation::Passed { aggregate, .. } = evaluation {
                aggregate.loss_confidence_95.upper += 0.25;
            }
        }),
        ("control effort", |evaluation| {
            if let CandidateEvaluation::Passed { aggregate, .. } = evaluation {
                aggregate.mean_control_effort += 0.1;
            }
        }),
    ];
    let sealed = sealed_campaign("aggregate-attack-fields");
    for (field, change) in cases {
        let mut evidence = sealed.clone();
        change_training_evaluation(&mut evidence, change);
        assert_refused(&evidence, field);
    }
}

#[test]
fn a_stored_run_loss_cannot_move_without_its_aggregate() {
    let mut evidence = sealed_campaign("aggregate-attack-run-loss");
    change_training_evaluation(&mut evidence, |evaluation| {
        if let CandidateEvaluation::Passed { runs, .. } = evaluation
            && let Some(run) = runs.first_mut()
        {
            run.loss += 0.5;
        }
    });

    assert_refused(&evidence, "run loss");
}

#[test]
fn an_objective_name_that_carries_whitespace_is_refused() {
    let sealed = sealed_campaign("aggregate-attack-names");
    for name in ["test.response ", " test.response", "test response"] {
        let mut evidence = sealed.clone();
        change_training_evaluation(&mut evidence, |evaluation| {
            if let CandidateEvaluation::Passed { runs, .. } = evaluation
                && let Some(run) = runs.first_mut()
                && let Some((key, value)) = run.objectives.pop_first()
            {
                let _ = key;
                run.objectives.insert(name.to_owned(), value);
            }
        });
        assert_refused(&evidence, name);
    }
}

#[test]
fn a_negative_objective_value_is_refused() {
    let mut evidence = sealed_campaign("aggregate-attack-negative-objective");
    change_training_evaluation(&mut evidence, |evaluation| {
        if let CandidateEvaluation::Passed { runs, .. } = evaluation
            && let Some(run) = runs.first_mut()
            && let Some(value) = run.objectives.values_mut().next()
        {
            *value = -1.0;
        }
    });

    assert_refused(&evidence, "negative objective");
}
