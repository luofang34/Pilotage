//! A later open inherits nothing from an open that did not commit.

use flight_tune::TuneError;

use super::super::TestDirectory;
use super::super::test_rig::FakeHandle;
use super::{expect_open_error, open_once};

#[test]
fn a_second_open_inherits_no_session_or_binding_from_a_failed_open() {
    let directory = TestDirectory::new("open-residue-none");
    let state = FakeHandle::new();
    state.0.borrow_mut().vehicle.fail_bind = true;

    expect_open_error(open_once(&directory, &state), "refuse the partial bind");

    assert!(!state.0.borrow().session_open);
    assert!(!state.0.borrow().vehicle.bound);
    let sessions_before = state.0.borrow().open_session_count;
    state.0.borrow_mut().vehicle.fail_bind = false;

    open_once(&directory, &state).expect("open after a failed attempt");

    // The second open took its own session rather than an inherited one.
    assert_eq!(
        state.0.borrow().open_session_count,
        sessions_before.wrapping_add(1)
    );
    assert!(state.0.borrow().session_open);
}

#[test]
fn a_second_open_reconciles_an_ambiguous_rollback_before_it_acquires() {
    let directory = TestDirectory::new("open-residue-ambiguous");
    let state = FakeHandle::new();
    {
        let mut fake = state.0.borrow_mut();
        fake.vehicle.fail_bind = true;
        // The reconciliation close is the first; the cleanup close is the
        // second, and it leaves the session in an uncertain state.
        fake.fail_session_close_on = Some(2);
    }

    let error = expect_open_error(open_once(&directory, &state), "refuse the partial bind");

    assert!(matches!(error, TuneError::OpenAndRollbackFailed { .. }));
    assert!(state.0.borrow().session_open);
    let resumed_at = state.0.borrow().open_order.len();
    state.0.borrow_mut().vehicle.fail_bind = false;

    open_once(&directory, &state).expect("open after an ambiguous rollback");

    let path = state.0.borrow().open_order[resumed_at..].to_vec();
    assert_eq!(
        path.first().map(String::as_str),
        Some("release_binding"),
        "the second open reconciled the vehicle binding first"
    );
    assert_eq!(
        path.get(1).map(String::as_str),
        Some("close_session"),
        "the second open closed the uncertain session next"
    );
    assert_eq!(
        path.get(2).map(String::as_str),
        Some("open_session"),
        "the second open acquired only after it proved absence"
    );
}

#[test]
fn an_open_acquires_nothing_until_the_prior_session_is_proven_absent() {
    let directory = TestDirectory::new("open-residue-fail-closed");
    let state = FakeHandle::new();
    state.0.borrow_mut().fail_session_close_on = Some(1);

    let error = expect_open_error(
        open_once(&directory, &state),
        "refuse an unproven prior session",
    );

    let TuneError::OpenNotReconciled { report } = error else {
        panic!("an unreconciled prior open did not fail closed");
    };
    assert!(!report.is_complete());
    assert_eq!(state.0.borrow().open_session_count, 0);
    assert_eq!(state.0.borrow().vehicle.bind_count, 0);
}
