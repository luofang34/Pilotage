//! The final bar refuses a run over a scoped limit and a run that states none.

use flight_tune::{FinalQualificationOutcome, PromotionDecision};

use super::test_rig::{FakeHandle, SequenceStrategy, TestDirectory};

#[test]
fn final_qualification_rejects_a_named_objective_limit() {
    let directory = TestDirectory::new("named-final-objective");
    let state = FakeHandle::new();
    let mut tuner = super::open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![1.0]),
        2.0,
    )
    .expect("open tuner");

    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");
    tuner.freeze_candidate().expect("freeze candidate");
    assert!(matches!(
        tuner.run_promotion_once_blocking().expect("run promotion"),
        PromotionDecision::Promoted { .. }
    ));
    assert_eq!(
        tuner
            .run_final_qualification_once_blocking()
            .expect("run final qualification"),
        FinalQualificationOutcome::FailedObjective {
            metric: "test.response".to_owned(),
        }
    );
    assert_eq!(state.0.borrow().vehicle.gain, 0.0);
}

/// The rig stage with one extra declared final objective and its scoped row.
fn stage_declaring(objective: &str) -> flight_tune::SearchStage {
    let mut stage = super::stage();
    stage.qualification.objectives.insert(objective.to_owned());
    let final_id = stage.final_qualification_scenarios[0].revision_id.clone();
    let mut rows = stage.response_targets.targets.clone();
    let mut extra = rows
        .iter()
        .find(|row| row.mission_revision_id == final_id)
        .expect("a final row exists")
        .clone();
    extra.objective = objective.to_owned();
    extra.limit = 1.0;
    rows.push(extra);
    stage.response_targets =
        flight_tune::ResponseTargetTable::new(rows).expect("the widened table is valid");
    stage
}

#[test]
fn final_qualification_rejects_a_missing_named_objective() {
    let directory = TestDirectory::new("missing-final-objective");
    let state = FakeHandle::new();
    // The declared objective gains a scoped row, so the stage stays complete
    // and the run is the only thing that does not state the value.
    let policy = stage_declaring("required.missing");
    let mut tuner = super::open_stage(
        directory.path(),
        state,
        SequenceStrategy::new(vec![0.5]),
        2.0,
        policy,
    )
    .expect("open tuner");

    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");
    tuner.freeze_candidate().expect("freeze candidate");
    assert!(matches!(
        tuner.run_promotion_once_blocking().expect("run promotion"),
        PromotionDecision::Promoted { .. }
    ));
    assert_eq!(
        tuner
            .run_final_qualification_once_blocking()
            .expect("run final qualification"),
        FinalQualificationOutcome::FailedObjective {
            metric: "required.missing".to_owned(),
        }
    );
}
