//! The open transaction owns every resource it acquires until it commits.

use flight_tune::{
    CampaignBackend, Digest, OpenRollbackOperation, SimulatorSessionAcquisition,
    SimulatorVehicleFactory, TuneError, VehicleBindingAcquisition, VehicleBindingRollback,
    conformance,
};

use super::open;
use super::test_rig::{FakeBackend, FakeFactory, FakeHandle, FakeRuntimeLease, SequenceStrategy};
use super::{TestDirectory, TestTuner};

#[path = "open_transaction/residue.rs"]
mod residue;

/// A session digest that no journal in these tests produces.
///
/// The rollback tests below drive the adapters directly, so they name a
/// session of their own rather than one an open transaction minted.
const STANDALONE_SESSION: Digest = Digest::from_bytes([17; 32]);

fn open_once(directory: &TestDirectory, state: &FakeHandle) -> Result<TestTuner, TuneError> {
    open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(Vec::new()),
        2.0,
    )
}

/// Returns the open path after the entries a completed open already wrote.
fn open_path_after(state: &FakeHandle, start: usize) -> Vec<String> {
    state.0.borrow().open_order[start..].to_vec()
}

/// Takes the failure of an open that must not return a tuner.
fn expect_open_error(result: Result<TestTuner, TuneError>, reason: &str) -> TuneError {
    match result {
        Ok(_) => panic!("the open completed: {reason}"),
        Err(error) => error,
    }
}

#[test]
fn a_simulator_receipt_mismatch_rolls_back_the_session_and_never_binds() {
    let directory = TestDirectory::new("open-session-receipt-mismatch");
    let state = FakeHandle::new();
    state.0.borrow_mut().bad_session_receipt = true;

    let error = expect_open_error(open_once(&directory, &state), "refuse the session receipt");

    assert!(matches!(error, TuneError::ReceiptMismatch { .. }));
    assert_eq!(
        open_path_after(&state, 0),
        [
            "release_binding",
            "close_session",
            "open_session",
            "close_session"
        ]
    );
    assert_eq!(state.0.borrow().vehicle.bind_count, 0);
    assert!(!state.0.borrow().session_open);
    assert!(!state.0.borrow().vehicle.bound);
}

#[test]
fn a_factory_bind_error_rolls_back_the_partial_binding_and_the_session() {
    let directory = TestDirectory::new("open-bind-error");
    let state = FakeHandle::new();
    state.0.borrow_mut().vehicle.fail_bind = true;

    let error = expect_open_error(open_once(&directory, &state), "refuse the partial bind");

    assert!(matches!(error, TuneError::Adapter { .. }));
    assert_eq!(
        open_path_after(&state, 2),
        ["open_session", "bind", "release_binding", "close_session"]
    );
    assert!(!state.0.borrow().session_open);
    assert!(!state.0.borrow().vehicle.bound);
}

#[test]
fn a_vehicle_binding_mismatch_rolls_back_the_binding_and_the_session() {
    let directory = TestDirectory::new("open-binding-receipt-mismatch");
    let state = FakeHandle::new();
    state.0.borrow_mut().vehicle.bad_binding_receipt = true;

    let error = expect_open_error(open_once(&directory, &state), "refuse the binding receipt");

    assert!(matches!(error, TuneError::ReceiptMismatch { .. }));
    assert_eq!(
        open_path_after(&state, 2),
        ["open_session", "bind", "release_binding", "close_session"]
    );
    assert!(!state.0.borrow().session_open);
    assert!(!state.0.borrow().vehicle.bound);
}

#[test]
fn a_later_open_check_error_rolls_back_both_open_resources() {
    let directory = TestDirectory::new("open-late-check-error");
    let state = FakeHandle::new();
    state
        .0
        .borrow_mut()
        .vehicle
        .bad_candidate_readback_on_ensure = Some(1);

    let error = expect_open_error(open_once(&directory, &state), "refuse the settled readback");

    assert!(matches!(error, TuneError::ReceiptMismatch { .. }));
    let path = open_path_after(&state, 2);
    assert_eq!(path.first().map(String::as_str), Some("open_session"));
    assert_eq!(
        path[path.len().wrapping_sub(2)..],
        ["release_binding", "close_session"]
    );
    assert!(!state.0.borrow().session_open);
    assert!(!state.0.borrow().vehicle.bound);
}

#[test]
fn cleanup_releases_the_vehicle_binding_before_the_simulator_session() {
    let directory = TestDirectory::new("open-reverse-order");
    let state = FakeHandle::new();
    state.0.borrow_mut().vehicle.fail_bind = true;

    expect_open_error(open_once(&directory, &state), "refuse the partial bind");

    let path = open_path_after(&state, 2);
    let release = path
        .iter()
        .position(|step| step == "release_binding")
        .expect("a vehicle release");
    let close = path
        .iter()
        .position(|step| step == "close_session")
        .expect("a session close");
    assert!(release < close);
}

#[test]
fn a_vehicle_rollback_error_does_not_stop_the_session_rollback() {
    let directory = TestDirectory::new("open-release-error");
    let state = FakeHandle::new();
    {
        let mut fake = state.0.borrow_mut();
        fake.vehicle.fail_bind = true;
        // The reconciliation release is the first; the cleanup release is
        // the second.
        fake.vehicle.fail_release_on = Some(2);
    }

    let error = expect_open_error(open_once(&directory, &state), "refuse the partial bind");

    let TuneError::OpenAndRollbackFailed { primary, rollback } = error else {
        panic!("a failed rollback did not reach the result");
    };
    assert!(matches!(*primary, TuneError::Adapter { .. }));
    assert_eq!(
        rollback.operations().collect::<Vec<_>>(),
        [
            OpenRollbackOperation::VehicleBinding,
            OpenRollbackOperation::SimulatorSession
        ]
    );
    assert_eq!(
        rollback
            .failures()
            .map(|(operation, _)| operation)
            .collect::<Vec<_>>(),
        [OpenRollbackOperation::VehicleBinding]
    );
    // The session closed even though the release before it failed.
    assert!(!state.0.borrow().session_open);
}

#[test]
fn one_result_preserves_the_primary_error_and_every_rollback_error() {
    let directory = TestDirectory::new("open-every-rollback-error");
    let state = FakeHandle::new();
    {
        let mut fake = state.0.borrow_mut();
        fake.vehicle.fail_bind = true;
        fake.vehicle.fail_release_on = Some(2);
        fake.fail_session_close_on = Some(2);
    }

    let error = expect_open_error(open_once(&directory, &state), "refuse the partial bind");

    let TuneError::OpenAndRollbackFailed { primary, rollback } = error else {
        panic!("the rollback failures did not reach the result");
    };
    let TuneError::Adapter { operation, .. } = *primary else {
        panic!("the primary failure did not reach the result");
    };
    assert_eq!(operation, "bind simulator vehicle");
    assert_eq!(
        rollback
            .failures()
            .map(|(operation, _)| operation)
            .collect::<Vec<_>>(),
        [
            OpenRollbackOperation::VehicleBinding,
            OpenRollbackOperation::SimulatorSession
        ]
    );
    assert!(!rollback.is_complete());
}

#[test]
fn a_repeated_rollback_has_the_same_result_and_no_further_effect() {
    let state = FakeHandle::new();
    let factory = FakeFactory::new(state.clone());
    let mut backend = FakeBackend::new(state.clone());
    let mut rollback = factory.rollback_handle();
    let session = backend.session_acquisition(STANDALONE_SESSION);
    let binding = factory.binding_acquisition(STANDALONE_SESSION);
    state.0.borrow_mut().vehicle.gain = 0.75;

    let first = (
        rollback.release_binding_blocking(&binding),
        backend.close_session_blocking(&session),
    );
    let second = (
        rollback.release_binding_blocking(&binding),
        backend.close_session_blocking(&session),
    );

    assert_eq!(first, second);
    assert!(first.0.is_ok() && first.1.is_ok());
    // A repeat writes nothing that a first release did not already write.
    assert_eq!(state.0.borrow().vehicle.gain, 0.75);
    assert_eq!(state.0.borrow().vehicle.apply_count, 0);
}

#[test]
fn a_rollback_refuses_a_foreign_acquisition_identity() {
    let state = FakeHandle::new();
    let factory = FakeFactory::new(state.clone());
    let mut backend = FakeBackend::new(state.clone());
    let mut rollback = factory.rollback_handle();
    assert!(backend.close_session_blocking(&foreign_session()).is_err());
    assert!(
        rollback
            .release_binding_blocking(&foreign_binding())
            .is_err()
    );
}

#[test]
fn the_reference_backend_and_factory_pass_the_open_rollback_suite() {
    let state = FakeHandle::new();
    let factory = FakeFactory::new(state.clone());
    let mut backend = FakeBackend::new(state.clone());
    let owned_session = backend.session_acquisition(STANDALONE_SESSION);
    let owned_binding = factory.binding_acquisition(STANDALONE_SESSION);
    let mut rollback = factory.rollback_handle();

    conformance::check_open_rollback(
        &mut backend,
        &mut rollback,
        &owned_session,
        &foreign_session(),
        &owned_binding,
        &foreign_binding(),
    )
    .expect("the reference adapters keep the open rollback contract");
}

#[test]
fn an_open_never_takes_a_second_runtime_lease_or_stops_the_operator_runtime() {
    let directory = TestDirectory::new("open-runtime-lease");
    let state = FakeHandle::new();
    {
        let mut fake = state.0.borrow_mut();
        fake.runtime_lease = Some(FakeRuntimeLease::acquired());
        fake.vehicle.fail_bind = true;
    }

    expect_open_error(open_once(&directory, &state), "refuse the partial bind");

    {
        let fake = state.0.borrow();
        let lease = fake.runtime_lease.as_ref().expect("the armed lease");
        assert_eq!(lease.acquisitions, 1);
        assert!(lease.held);
    }
    state.0.borrow_mut().vehicle.fail_bind = false;

    // The operator runtime survived the cleanup, so a later open still has
    // one to open a session against.
    open_once(&directory, &state).expect("open after a failed attempt");

    let fake = state.0.borrow();
    let lease = fake.runtime_lease.as_ref().expect("the armed lease");
    assert_eq!(lease.acquisitions, 1);
    assert!(lease.held);
}

/// Names a session no reference backend in these tests answers for.
fn foreign_session() -> SimulatorSessionAcquisition {
    SimulatorSessionAcquisition::new(
        STANDALONE_SESSION,
        Digest::from_bytes([29; 32]),
        Digest::from_bytes([31; 32]),
    )
}

/// Names a binding no reference factory in these tests answers for.
fn foreign_binding() -> VehicleBindingAcquisition {
    VehicleBindingAcquisition::new(
        STANDALONE_SESSION,
        Digest::from_bytes([37; 32]),
        Digest::from_bytes([41; 32]),
    )
}
