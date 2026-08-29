//! The exact link from one search group to its own training evidence.

use flight_tune::{AttemptRole, JournalEvent, TuneError, Tuner};

use super::TestTuner;
use super::test_rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, ParameterSequenceStrategy,
    QuadraticMetric, TestDirectory, two_group_candidate, two_group_stage,
};

/// The run counts each declared suite states.
///
/// The direct suite flies one mission twice. The operator suite flies its own
/// mission and one direct guard mission twice each.
const DIRECT_RUNS: usize = 2;
const OPERATOR_RUNS: usize = 4;

#[test]
fn each_group_takes_its_own_suite_and_its_own_incumbent_baseline() {
    let directory = TestDirectory::new("training-suite-per-group");
    let state = FakeHandle::new();
    let mut tuner = open(
        &directory,
        state.clone(),
        ParameterSequenceStrategy::new(vec![vec![("gain", 0.4)], vec![("trim", 0.4)]]),
    )
    .expect("open a two group tuner");

    tuner
        .run_training_attempts_blocking(2)
        .expect("run one challenger for each group");

    assert_eq!(
        prepared_roles(&tuner),
        vec![
            AttemptRole::TrainingBaseline { suite_index: 0 },
            AttemptRole::TrainingChallenger {
                attempt_index: 0,
                suite_index: 0,
            },
            AttemptRole::TrainingBaseline { suite_index: 1 },
            AttemptRole::TrainingChallenger {
                attempt_index: 1,
                suite_index: 1,
            },
        ],
        "each group takes its own suite, and each suite takes its own baseline"
    );
    assert_eq!(
        prepared_run_counts(&tuner),
        vec![DIRECT_RUNS, DIRECT_RUNS, OPERATOR_RUNS, OPERATOR_RUNS],
        "an attempt flies exactly the missions its own suite declares"
    );
}

#[test]
fn an_unchanged_incumbent_reuses_the_exact_suite_baseline() {
    let directory = TestDirectory::new("training-suite-baseline-reuse");
    let state = FakeHandle::new();
    let mut tuner = open(
        &directory,
        state.clone(),
        // The first proposal does not beat the starting candidate, so the
        // incumbent does not move and its baseline still answers the second.
        ParameterSequenceStrategy::new(vec![vec![("gain", 2.0)], vec![("gain", 1.9)]]),
    )
    .expect("open a two group tuner");

    tuner
        .run_training_attempts_blocking(2)
        .expect("run two challengers on one suite");

    assert_eq!(
        prepared_roles(&tuner)
            .iter()
            .filter(|role| matches!(role, AttemptRole::TrainingBaseline { .. }))
            .count(),
        1,
        "an unchanged incumbent keeps its one baseline for the suite"
    );
}

#[test]
fn a_changed_incumbent_takes_a_new_baseline_on_the_same_suite() {
    let directory = TestDirectory::new("training-suite-baseline-refresh");
    let state = FakeHandle::new();
    let mut tuner = open(
        &directory,
        state.clone(),
        // The first proposal wins, so the second challenger compares against a
        // candidate that has never run this suite.
        ParameterSequenceStrategy::new(vec![vec![("gain", 1.6)], vec![("gain", 1.2)]]),
    )
    .expect("open a two group tuner");

    tuner
        .run_training_attempts_blocking(2)
        .expect("run two challengers on one suite");

    let prepared = prepared_attempts(&tuner);
    let baselines = prepared
        .iter()
        .filter(|(role, _)| matches!(role, AttemptRole::TrainingBaseline { suite_index: 0 }))
        .map(|(_, candidate)| *candidate)
        .collect::<Vec<_>>();
    let first_challenger = prepared
        .iter()
        .find(|(role, _)| {
            matches!(
                role,
                AttemptRole::TrainingChallenger {
                    attempt_index: 0,
                    ..
                }
            )
        })
        .map(|(_, candidate)| *candidate)
        .expect("the first challenger");

    assert_eq!(
        baselines.len(),
        2,
        "a changed incumbent owes a new baseline"
    );
    assert_eq!(
        baselines[1], first_challenger,
        "the second baseline states the exact new incumbent"
    );
}

#[test]
fn a_proposal_that_changes_two_groups_fails_before_external_mutation() {
    let directory = TestDirectory::new("training-suite-two-groups");
    let state = FakeHandle::new();
    let mut tuner = open(
        &directory,
        state.clone(),
        ParameterSequenceStrategy::new(vec![vec![("gain", 0.4), ("trim", 0.4)]]),
    )
    .expect("open a two group tuner");
    tuner
        .run_training_attempts_blocking(0)
        .expect("settle the starting baseline");
    let journal_length = tuner.journal().entries().len();
    let runs = state.0.borrow().scenario_runs.len();

    let error = tuner
        .run_training_attempts_blocking(1)
        .expect_err("refuse a proposal that changes two groups");

    assert!(matches!(error, TuneError::InvalidCandidate { .. }));
    assert_eq!(tuner.journal().entries().len(), journal_length);
    assert_eq!(state.0.borrow().scenario_runs.len(), runs);
}

#[test]
fn a_restart_resumes_the_same_suite_and_run_plan() {
    let directory = TestDirectory::new("training-suite-restart");
    let state = FakeHandle::new();
    let strategy = ParameterSequenceStrategy::new(vec![vec![("gain", 0.4)], vec![("trim", 0.4)]]);
    let mut tuner = open(&directory, state.clone(), strategy).expect("open a two group tuner");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run the first group");
    // The second group owes a four-run suite baseline before its challenger,
    // so the stop lands inside the challenger and not inside the baseline.
    let prepared_so_far = state.0.borrow().prepare_count;
    state.0.borrow_mut().panic_on_prepare = Some(prepared_so_far.wrapping_add(5));
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tuner.run_training_attempts_blocking(1).ok();
    }));
    assert!(stopped.is_err(), "the second group stopped inside a run");
    drop(tuner);
    state.0.borrow_mut().panic_on_prepare = None;

    let mut resumed = open(
        &directory,
        state.clone(),
        ParameterSequenceStrategy::new(vec![vec![("gain", 0.4)], vec![("trim", 0.4)]]),
    )
    .expect("resume the same campaign");
    resumed
        .run_training_attempts_blocking(1)
        .expect("finish the second group");

    let roles = prepared_roles(&resumed);
    assert!(
        roles.iter().all(|role| !matches!(
            role,
            AttemptRole::TrainingChallenger { suite_index: 0, .. }
        ) || matches!(
            role,
            AttemptRole::TrainingChallenger {
                attempt_index: 0,
                ..
            }
        )),
        "a resumed campaign keeps each challenger on the suite it started"
    );
    assert_eq!(
        roles
            .iter()
            .filter(|role| matches!(role, AttemptRole::TrainingChallenger { suite_index: 1, .. }))
            .count(),
        1,
        "the interrupted challenger resumed instead of proposing again"
    );
}

/// A suite narrows the search and nothing else.
///
/// Promotion and final qualification decide what ships, so they read their
/// complete hidden partitions whatever suite the search used.
#[test]
fn promotion_and_final_qualification_keep_their_complete_partitions() {
    let directory = TestDirectory::new("training-suite-hidden-partitions");
    let state = FakeHandle::new();
    let stage = two_group_stage();
    let mut tuner = open(
        &directory,
        state.clone(),
        ParameterSequenceStrategy::new(vec![vec![("gain", 0.5)]]),
    )
    .expect("open a two group tuner");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run one challenger");
    tuner.freeze_candidate().expect("freeze the champion");
    tuner
        .run_promotion_once_blocking()
        .expect("run the promotion decision");
    tuner
        .run_final_qualification_once_blocking()
        .expect("run final qualification");

    let hidden = prepared_attempts(&tuner)
        .into_iter()
        .zip(prepared_run_counts(&tuner))
        .filter(|((role, _), _)| role.training_suite_index().is_none())
        .map(|(_, runs)| runs)
        .collect::<Vec<_>>();
    let promotion = stage.promotion_scenarios.len() * stage.repetitions as usize;
    let qualification = stage.final_qualification_scenarios.len() * stage.repetitions as usize;

    assert_eq!(
        hidden,
        vec![promotion, promotion, qualification],
        "each hidden attempt flies its complete partition"
    );
}

fn open(
    directory: &TestDirectory,
    state: FakeHandle,
    strategy: ParameterSequenceStrategy,
) -> Result<TestTuner<ParameterSequenceStrategy>, TuneError> {
    Tuner::open_or_resume(
        directory.path(),
        two_group_stage(),
        91,
        two_group_candidate(0.0, 0.0),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        EnvelopeGates::new(2.0),
        QuadraticMetric::new(state),
        strategy,
    )
}

fn prepared_roles<P: flight_tune::ProposalStrategy>(tuner: &TestTuner<P>) -> Vec<AttemptRole> {
    prepared_attempts(tuner)
        .into_iter()
        .map(|(role, _)| role)
        .collect()
}

fn prepared_attempts<P: flight_tune::ProposalStrategy>(
    tuner: &TestTuner<P>,
) -> Vec<(AttemptRole, flight_tune::Digest)> {
    tuner
        .journal()
        .entries()
        .iter()
        .filter_map(|entry| match entry.event {
            JournalEvent::AttemptPrepared {
                role, candidate, ..
            } => Some((role, candidate)),
            _ => None,
        })
        .collect()
}

/// The prepared run count of each attempt, in attempt order.
fn prepared_run_counts<P: flight_tune::ProposalStrategy>(tuner: &TestTuner<P>) -> Vec<usize> {
    let mut counts = Vec::new();
    for entry in tuner.journal().entries() {
        match entry.event {
            JournalEvent::AttemptPrepared { .. } => counts.push(0_usize),
            JournalEvent::RunPrepared { .. } => {
                if let Some(last) = counts.last_mut() {
                    *last = last.wrapping_add(1);
                }
            }
            _ => {}
        }
    }
    counts
}
