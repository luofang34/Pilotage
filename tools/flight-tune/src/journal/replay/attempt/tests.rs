#![allow(clippy::expect_used, clippy::panic)]

use pilotage_trial::Digest;

use crate::AttemptRole;

use super::super::initial_state;
use super::super::super::retry::AuthorizedRetry;
use super::prepare;

/// A replacement execution repeats one condition, so it cannot change the
/// suite that condition ran on.
#[test]
fn a_replacement_that_changes_its_suite_is_refused() {
    let stage = crate::model::training_suite::tests::stage_for_budget();
    let candidate = Digest::from_bytes([5; 32]);
    let source = AttemptRole::TrainingBaseline { suite_index: 0 };
    let replacement = AttemptRole::TrainingBaseline { suite_index: 1 };
    let mut state = initial_state(candidate);
    state.next_trial_id = 1;
    state.authorized_retry = Some(AuthorizedRetry {
        source_trial_id: 0,
        replacement_trial_id: 1,
        retry_index: 1,
        role: source,
        candidate,
        plan_digest: source
            .plan_digest(&stage, candidate, 91)
            .expect("the source plan digest"),
        transition: None,
    });

    let error = prepare(
        &mut state,
        1,
        replacement,
        candidate,
        replacement
            .plan_digest(&stage, candidate, 91)
            .expect("the replacement plan digest"),
        None,
        &stage,
        candidate,
        91,
    )
    .expect_err("refuse a replacement on another suite");

    assert!(matches!(error, crate::TuneError::InvalidJournal { .. }));
    assert!(state.pending.is_none());
}

/// A replacement that keeps its suite and its run plan is authorized.
#[test]
fn a_replacement_that_keeps_its_suite_is_prepared() {
    let stage = crate::model::training_suite::tests::stage_for_budget();
    let candidate = Digest::from_bytes([5; 32]);
    let role = AttemptRole::TrainingBaseline { suite_index: 1 };
    let plan_digest = role
        .plan_digest(&stage, candidate, 91)
        .expect("the plan digest");
    let mut state = initial_state(candidate);
    state.next_trial_id = 1;
    state.authorized_retry = Some(AuthorizedRetry {
        source_trial_id: 0,
        replacement_trial_id: 1,
        retry_index: 1,
        role,
        candidate,
        plan_digest,
        transition: None,
    });

    prepare(
        &mut state, 1, role, candidate, plan_digest, None, &stage, candidate, 91,
    )
    .expect("prepare the authorized replacement");

    assert_eq!(
        state.pending.as_ref().map(|pending| pending.retry_index),
        Some(1)
    );
}
