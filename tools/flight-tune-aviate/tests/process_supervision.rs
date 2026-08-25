//! Public process-supervision lifecycle tests with real child processes.

#![allow(clippy::expect_used, clippy::panic)]

use flight_tune_aviate::{
    AviateSupervisorError, PreparedAviateProcess, RecoveryOutcome, SupervisionAttestation,
    recover_supervised_process_blocking,
};

#[path = "process_supervision/recovery_cases.rs"]
mod recovery_cases;
#[path = "process_supervision/support.rs"]
mod support;

use support::{DescendantControl, DriverProcess, FifoWatch, TestLaunch, UnrelatedProcess};

#[test]
fn supervised_target_fixture() {
    support::run_target_fixture();
}

#[test]
fn supervision_driver_fixture() {
    support::run_driver_fixture();
}

#[test]
fn unrelated_process_fixture() {
    support::run_unrelated_fixture();
}

#[test]
fn stubborn_descendant_fixture() {
    support::run_stubborn_descendant_fixture();
}

#[test]
fn prepared_launch_requires_release_and_cleans_exact_target() {
    let fixture = TestLaunch::new("release");
    let target = FifoWatch::new(fixture.target_fifo());
    let prepared =
        PreparedAviateProcess::prepare_blocking(fixture.request()).expect("prepare launch");
    target.expect_no_event("the target remains behind the launch gate");
    let encoded = serde_json::to_vec(prepared.supervision_attestation()).expect("encode evidence");
    let persisted: SupervisionAttestation =
        serde_json::from_slice(&encoded).expect("read external evidence");
    assert_eq!(&persisted, prepared.supervision_attestation());

    let mut managed = prepared.release_blocking().expect("release exact target");
    target.expect_open("the released target starts");
    managed
        .ensure_running_blocking()
        .expect("target is running");
    let outcome = managed.terminate_blocking().expect("clean exact target");

    target.expect_eof("the exact target stops");
    assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));
}

#[test]
fn prepared_launch_can_close_without_starting_target() {
    let fixture = TestLaunch::new("cancel");
    let target = FifoWatch::new(fixture.target_fifo());
    let prepared =
        PreparedAviateProcess::prepare_blocking(fixture.request()).expect("prepare launch");

    let outcome = prepared.cancel_blocking().expect("cancel prepared launch");

    assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));
    target.expect_unused("cancel keeps the target closed");
}

#[test]
fn cleanup_kills_a_stubborn_same_group_descendant() {
    let fixture = TestLaunch::new("descendant");
    let target = FifoWatch::new(fixture.target_fifo());
    let descendant = FifoWatch::new(fixture.descendant_fifo());
    let mut control = DescendantControl::new(fixture.descendant_control_fifo());
    let prepared = PreparedAviateProcess::prepare_blocking(fixture.descendant_request())
        .expect("prepare descendant launch");
    let mut managed = prepared
        .release_blocking()
        .expect("release descendant launch");
    target.expect_open("the target starts");
    descendant.expect_open("the stubborn descendant starts");
    control.connect();

    let mut initial_error = None;
    let outcome = match managed.terminate_blocking() {
        Ok(outcome) => outcome,
        Err(error) => {
            initial_error = Some(error);
            control.release();
            managed
                .terminate_blocking()
                .expect("clean the group after the fixture releases its descendant")
        }
    };

    control.release();
    target.expect_eof("the exact target stops");
    descendant.expect_eof("the stubborn descendant stops");
    if let Some(error) = initial_error {
        panic!("production cleanup required the fixture fallback: {error}");
    }
    assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));
}

#[test]
fn driver_crash_before_release_never_starts_the_target() {
    let fixture = TestLaunch::new("driver-crash");
    let target = FifoWatch::new(fixture.target_fifo());
    let lifecycle = FifoWatch::new(fixture.lifecycle_fifo());
    let mut driver = DriverProcess::spawn(&fixture);
    lifecycle.expect_open("the driver process family starts");
    let attestation = driver.read_attestation();
    target.expect_no_event("the prepared driver keeps the target closed");

    driver.kill_and_wait();
    lifecycle.expect_eof("the owner and launch gate stop after driver failure");
    let outcome = recover_supervised_process_blocking(&attestation.recovery_request)
        .expect("read terminal evidence after driver failure");

    assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));
    target.expect_unused("driver failure does not start the target");
}

#[test]
fn driver_crash_after_release_cleans_the_complete_group() {
    let fixture = TestLaunch::new("released-driver-crash");
    let target = FifoWatch::new(fixture.target_fifo());
    let descendant = FifoWatch::new(fixture.descendant_fifo());
    let mut control = DescendantControl::new(fixture.descendant_control_fifo());
    let lifecycle = FifoWatch::new(fixture.lifecycle_fifo());
    let mut driver = DriverProcess::spawn_released_descendant(&fixture);
    lifecycle.expect_open("the released driver process family starts");
    let attestation = driver.read_attestation();
    target.expect_open("the released driver target starts");
    descendant.expect_open("the released driver descendant starts");
    control.connect();

    driver.kill_and_wait();
    target.expect_eof("driver failure stops the target");
    descendant.expect_eof("driver failure stops the descendant");
    lifecycle.expect_eof("driver failure stops the complete process family");
    control.release();
    let outcome = recover_supervised_process_blocking(&attestation.recovery_request)
        .expect("read terminal evidence after released driver failure");

    assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));
}

#[test]
fn owner_sigkill_after_release_recovers_the_complete_group() {
    let fixture = TestLaunch::new("owner-sigkill-released");
    let target = FifoWatch::new(fixture.target_fifo());
    let descendant = FifoWatch::new(fixture.descendant_fifo());
    let mut control = DescendantControl::new(fixture.descendant_control_fifo());
    let prepared = PreparedAviateProcess::prepare_blocking(fixture.descendant_request())
        .expect("prepare owner crash launch");
    let mut managed = prepared
        .release_blocking()
        .expect("release owner crash launch");
    target.expect_open("the owner crash target starts");
    descendant.expect_open("the owner crash descendant starts");
    control.connect();
    managed
        .ensure_running_blocking()
        .expect("the process family is live");

    support::kill_process(managed.supervision_attestation().supervisor_identity.pid);
    target.expect_eof("the gate stops the target after owner failure");
    descendant.expect_eof("the gate stops the descendant after owner failure");
    control.release();
    let outcome = managed
        .terminate_blocking()
        .expect("same-boot recovery publishes terminal evidence");

    assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));
}

#[test]
fn drop_reaps_a_sigkilled_owner_before_recovery() {
    let fixture = TestLaunch::new("owner-sigkill-drop");
    let target = FifoWatch::new(fixture.target_fifo());
    let prepared =
        PreparedAviateProcess::prepare_blocking(fixture.request()).expect("prepare owner drop run");
    let managed = prepared
        .release_blocking()
        .expect("release owner drop target");
    let attestation = managed.supervision_attestation().clone();
    target.expect_open("the owner drop target starts");

    support::kill_process(attestation.supervisor_identity.pid);
    target.expect_eof("the failed owner closes the exact target");
    drop(managed);
    let outcome = recover_supervised_process_blocking(&attestation.recovery_request)
        .expect("recovery continues after the owner is reaped");

    assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));
}

#[test]
fn gate_sigkill_after_release_uses_owner_fallback() {
    let fixture = TestLaunch::new("gate-sigkill-released");
    let target = FifoWatch::new(fixture.target_fifo());
    let descendant = FifoWatch::new(fixture.descendant_fifo());
    let mut control = DescendantControl::new(fixture.descendant_control_fifo());
    let prepared = PreparedAviateProcess::prepare_blocking(fixture.descendant_request())
        .expect("prepare gate crash launch");
    let mut managed = prepared
        .release_blocking()
        .expect("release gate crash launch");
    target.expect_open("the gate crash target starts");
    descendant.expect_open("the gate crash descendant starts");
    control.connect();

    support::kill_process(managed.supervision_attestation().target_gate_identity.pid);
    let outcome = managed
        .terminate_blocking()
        .expect("the owner fallback publishes terminal evidence");
    control.release();
    target.expect_eof("the owner fallback stops the target");
    descendant.expect_eof("the owner fallback stops the descendant");

    assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));
}

#[test]
fn gate_sigkill_before_release_keeps_target_closed() {
    let fixture = TestLaunch::new("gate-sigkill-prepared");
    let target = FifoWatch::new(fixture.target_fifo());
    let prepared =
        PreparedAviateProcess::prepare_blocking(fixture.request()).expect("prepare gate crash");
    support::kill_process(prepared.supervision_attestation().target_gate_identity.pid);

    let outcome = prepared
        .cancel_blocking()
        .expect("the owner records pre-release gate cleanup");

    assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));
    target.expect_unused("a failed gate cannot start its target");
}

#[test]
fn target_process_group_escape_is_detected_and_contained() {
    let fixture = TestLaunch::new("target-group-escape");
    let target = FifoWatch::new(fixture.target_fifo());
    let mut request = fixture.request();
    request
        .target_environment
        .insert(support::TARGET_ESCAPE_GROUP_ENV.to_owned(), "1".to_owned());
    let prepared =
        PreparedAviateProcess::prepare_blocking(request).expect("prepare escaped target");
    let recovery = prepared.supervision_attestation().recovery_request.clone();

    let error = match prepared.release_blocking() {
        Err(error) => {
            target.expect_open("the target attempts to leave the authorized group");
            target.expect_eof("the launch gate contains the rejected exact target");
            error
        }
        Ok(mut managed) => {
            target.expect_open("the released target leaves the authorized group");
            let error = managed
                .ensure_running_blocking()
                .expect_err("the runtime identity check rejects the escaped target");
            let outcome = managed
                .terminate_blocking()
                .expect("contain the escaped target");
            assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));
            target.expect_eof("the launch gate contains the escaped exact target");
            error
        }
    };

    assert!(
        matches!(error, AviateSupervisorError::IdentityMismatch { .. }),
        "unexpected escaped-target error: {error:?}"
    );
    let outcome =
        recover_supervised_process_blocking(&recovery).expect("read escape cleanup evidence");
    assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));
}
