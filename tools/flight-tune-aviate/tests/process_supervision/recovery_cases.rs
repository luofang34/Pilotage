use std::os::unix::fs::MetadataExt as _;

use super::*;

#[test]
fn recovery_repairs_linked_receipt_and_cleans_orphan_temporary() {
    let fixture = TestLaunch::new("receipt-repair");
    let prepared =
        PreparedAviateProcess::prepare_blocking(fixture.request()).expect("prepare repair run");
    let recovery = prepared.supervision_attestation().recovery_request.clone();
    let initial = prepared
        .cancel_blocking()
        .expect("publish initial terminal receipt");
    let receipt = fixture
        .storage_root()
        .join("supervisor-terminal-receipt.json");
    let linked = support::add_linked_temporary(&receipt, 1);
    let orphan = support::add_unlinked_temporary(fixture.storage_root(), 2);
    assert_eq!(
        std::fs::metadata(&receipt)
            .expect("inspect receipt")
            .nlink(),
        2
    );

    let replay =
        recover_supervised_process_blocking(&recovery).expect("repair terminal publication");

    assert_eq!(replay, initial);
    assert_eq!(
        std::fs::metadata(receipt).expect("inspect repair").nlink(),
        1
    );
    assert!(!linked.exists(), "the linked temporary is removed");
    assert!(!orphan.exists(), "the unlinked temporary is removed");
}

#[test]
fn invalid_recovery_does_not_repair_optional_documents() {
    let fixture = TestLaunch::new("invalid-recovery-mutation");
    let target_watch = FifoWatch::new(fixture.target_fifo());
    let prepared =
        PreparedAviateProcess::prepare_blocking(fixture.request()).expect("prepare mutation proof");
    let attestation = prepared.supervision_attestation().clone();
    let recovery = attestation.recovery_request.clone();
    let mut managed = prepared
        .release_blocking()
        .expect("release mutation-proof target");
    target_watch.expect_open("the mutation-proof target starts");
    let initial = managed
        .terminate_blocking()
        .expect("publish mutation proof");
    target_watch.expect_eof("the mutation-proof target stops");
    let receipt = fixture
        .storage_root()
        .join("supervisor-terminal-receipt.json");
    let target = fixture
        .storage_root()
        .join("supervisor-target-attestation.json");
    let linked_receipt = support::add_linked_temporary(&receipt, 3);
    let linked_target = support::add_linked_temporary(&target, 4);
    let boot_receipt =
        support::add_conflicting_recovery_receipt(fixture.storage_root(), &attestation);
    let linked_boot_receipt = support::add_linked_temporary(&boot_receipt, 5);
    let orphan = support::add_unlinked_temporary(fixture.storage_root(), 6);
    for invalid in changed_recovery_requests(&recovery) {
        let error = recover_supervised_process_blocking(&invalid)
            .expect_err("reject recovery before optional repair");
        assert!(matches!(
            error,
            AviateSupervisorError::InvalidDocument { .. }
                | AviateSupervisorError::InvalidRequest { .. }
        ));
    }
    assert_optional_publications_unrepaired(
        &[&receipt, &target, &boot_receipt],
        &[
            &linked_receipt,
            &linked_target,
            &linked_boot_receipt,
            &orphan,
        ],
    );

    std::fs::remove_file(boot_receipt).expect("remove test boot receipt");
    std::fs::remove_file(linked_boot_receipt).expect("remove linked test boot receipt");

    let replay = recover_supervised_process_blocking(&recovery)
        .expect("valid recovery repairs optional documents");
    assert_eq!(replay, initial);
    assert!(
        !linked_receipt.exists(),
        "valid recovery removes the linked terminal receipt"
    );
    assert!(
        !linked_target.exists(),
        "valid recovery removes the linked target attestation"
    );
    assert!(
        !orphan.exists(),
        "valid recovery removes the orphan temporary"
    );
}

fn assert_optional_publications_unrepaired(
    publications: &[&std::path::Path],
    temporaries: &[&std::path::Path],
) {
    for publication in publications {
        assert_eq!(
            std::fs::metadata(publication)
                .expect("inspect optional publication")
                .nlink(),
            2
        );
    }
    for temporary in temporaries {
        assert!(
            temporary.exists(),
            "the optional temporary remains unchanged"
        );
    }
}

fn changed_recovery_requests(
    recovery: &flight_tune_aviate::RecoveryRequest,
) -> Vec<flight_tune_aviate::RecoveryRequest> {
    let mut schema = recovery.clone();
    schema.schema_version = schema.schema_version.wrapping_add(1);
    let mut run = recovery.clone();
    run.run_intent_digest = support::digest_bytes(b"wrong run intent");
    let mut supervisor = recovery.clone();
    supervisor.supervisor_executable_digest = support::digest_bytes(b"wrong supervisor");
    let mut target = recovery.clone();
    target.target_executable_digest = support::digest_bytes(b"wrong target");
    let mut spawn = recovery.clone();
    spawn.expected_spawn_intent_digest = support::digest_bytes(b"wrong spawn intent");
    let mut process = recovery.clone();
    process.expected_process_identity_digest = support::digest_bytes(b"wrong process identity");
    let mut timeout = recovery.clone();
    timeout.cleanup_timeout_millis = timeout.cleanup_timeout_millis.wrapping_add(1);
    vec![schema, run, supervisor, target, spawn, process, timeout]
}

#[test]
fn outcome_replay_rechecks_runtime_cleanup() {
    let fixture = TestLaunch::new("outcome-replay-cleanup");
    let prepared =
        PreparedAviateProcess::prepare_blocking(fixture.request()).expect("prepare replay run");
    let recovery = prepared.supervision_attestation().recovery_request.clone();
    let initial = prepared
        .cancel_blocking()
        .expect("publish replay terminal receipt");
    let replay =
        recover_supervised_process_blocking(&recovery).expect("replay verifies empty resources");
    assert_eq!(replay, initial);
    let socket = fixture.runtime_root().join("parent-ready.sock");
    let listener =
        std::os::unix::net::UnixListener::bind(&socket).expect("bind a replacement runtime socket");
    drop(listener);

    let error =
        recover_supervised_process_blocking(&recovery).expect_err("reject replacement socket");
    assert!(matches!(
        error,
        AviateSupervisorError::InvalidRequest { .. }
    ));
    assert!(
        socket.exists(),
        "replay does not remove a replacement socket"
    );
    std::fs::remove_file(&socket).expect("clean test-only replacement socket");
    let unknown = fixture.runtime_root().join("unknown-entry");
    std::fs::write(&unknown, b"unknown").expect("create unknown runtime entry");
    let error =
        recover_supervised_process_blocking(&recovery).expect_err("reject unknown runtime entry");
    assert!(matches!(
        error,
        AviateSupervisorError::InvalidRequest { .. }
    ));
    assert!(
        unknown.exists(),
        "recovery does not remove an unknown entry"
    );
    std::fs::remove_file(unknown).expect("clean test-only unknown entry");
}

#[test]
fn recovery_rejects_conflicting_terminal_and_boot_receipts() {
    let fixture = TestLaunch::new("conflicting-outcomes");
    let prepared =
        PreparedAviateProcess::prepare_blocking(fixture.request()).expect("prepare conflict run");
    let attestation = prepared.supervision_attestation().clone();
    let _terminal = prepared
        .cancel_blocking()
        .expect("publish terminal receipt");
    support::add_conflicting_recovery_receipt(fixture.storage_root(), &attestation);

    let error = recover_supervised_process_blocking(&attestation.recovery_request)
        .expect_err("reject two durable outcomes");

    assert!(matches!(
        error,
        AviateSupervisorError::InvalidDocument {
            document: "process outcome",
            ..
        }
    ));
}

#[test]
fn terminal_replay_binds_digest_canonical_bytes_and_closed_set() {
    let fixture = TestLaunch::new("terminal-replay-validation");
    let prepared =
        PreparedAviateProcess::prepare_blocking(fixture.request()).expect("prepare replay proof");
    let recovery = prepared.supervision_attestation().recovery_request.clone();
    let outcome = prepared.cancel_blocking().expect("publish replay proof");
    let receipt = fixture
        .storage_root()
        .join("supervisor-terminal-receipt.json");
    let bytes = std::fs::read(&receipt).expect("read terminal receipt bytes");
    let receipt_digest = match outcome {
        RecoveryOutcome::Terminal { receipt_digest } => receipt_digest,
        RecoveryOutcome::BootChange { .. } => panic!("same-boot cancel returns terminal evidence"),
    };
    assert_eq!(receipt_digest, support::digest_bytes(&bytes));

    let mut invalid_request = recovery.clone();
    invalid_request.expected_process_identity_digest = support::digest_bytes(b"wrong process");
    let error =
        recover_supervised_process_blocking(&invalid_request).expect_err("reject wrong digest");
    assert!(matches!(
        error,
        AviateSupervisorError::InvalidDocument {
            document: "supervision attestation",
            ..
        }
    ));

    let unknown = support::add_unknown_storage_object(fixture.storage_root());
    let error =
        recover_supervised_process_blocking(&recovery).expect_err("reject unknown storage object");
    assert!(matches!(
        error,
        AviateSupervisorError::InvalidDocument { .. }
    ));
    assert!(
        unknown.exists(),
        "recovery does not remove an unknown object"
    );
    std::fs::remove_file(unknown).expect("clean test-only unknown object");

    let mut noncanonical = bytes;
    noncanonical.push(b'\n');
    support::replace_file_bytes(&receipt, &noncanonical);
    let error =
        recover_supervised_process_blocking(&recovery).expect_err("reject noncanonical receipt");
    assert!(matches!(
        error,
        AviateSupervisorError::InvalidDocument {
            document: "supervisor-terminal-receipt.json",
            ..
        }
    ));
}

#[test]
fn invalid_recovery_does_not_signal_live_processes() {
    let fixture = TestLaunch::new("invalid-recovery");
    let target = FifoWatch::new(fixture.target_fifo());
    let unrelated_watch = FifoWatch::new(fixture.unrelated_fifo());
    let mut unrelated = UnrelatedProcess::spawn(fixture.unrelated_fifo());
    unrelated_watch.expect_open("the unrelated process starts");
    let prepared =
        PreparedAviateProcess::prepare_blocking(fixture.request()).expect("prepare launch");
    let mut managed = prepared.release_blocking().expect("release exact target");
    target.expect_open("the supervised target starts");
    let mut invalid = managed.supervision_attestation().recovery_request.clone();
    invalid.expected_process_identity_digest = support::digest_bytes(b"wrong process");

    let error = recover_supervised_process_blocking(&invalid).expect_err("reject invalid recovery");

    assert!(matches!(error, AviateSupervisorError::SupervisorActive));
    managed
        .ensure_running_blocking()
        .expect("the supervised target remains live");
    unrelated.expect_running();
    unrelated.stop_and_wait();
    unrelated_watch.expect_eof("the unrelated process exits only on request");
    let outcome = managed
        .terminate_blocking()
        .expect("clean supervised target");
    assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));
    target.expect_eof("the supervised target stops on request");
}

#[test]
fn released_writer_rejects_changed_authorization_without_signaling() {
    let fixture = TestLaunch::new("changed-recovery-authorization");
    let unrelated_watch = FifoWatch::new(fixture.unrelated_fifo());
    let mut unrelated = UnrelatedProcess::spawn(fixture.unrelated_fifo());
    unrelated_watch.expect_open("the unrelated process starts");
    let prepared =
        PreparedAviateProcess::prepare_blocking(fixture.request()).expect("prepare launch");
    let recovery = prepared.supervision_attestation().recovery_request.clone();
    let outcome = prepared
        .cancel_blocking()
        .expect("release the recovery writer");
    assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));

    let mut changed_run = recovery.clone();
    changed_run.run_intent_digest = support::digest_bytes(b"changed run intent");
    let mut changed_supervisor = recovery.clone();
    changed_supervisor.supervisor_executable_digest =
        support::digest_bytes(b"changed supervisor executable");
    let mut changed_target = recovery.clone();
    changed_target.target_executable_digest = support::digest_bytes(b"changed target executable");
    let mut changed_process = recovery;
    changed_process.expected_process_identity_digest =
        support::digest_bytes(b"changed process identity");

    for invalid in [
        changed_run,
        changed_supervisor,
        changed_target,
        changed_process,
    ] {
        let error = recover_supervised_process_blocking(&invalid)
            .expect_err("reject changed recovery authorization");
        assert!(matches!(
            error,
            AviateSupervisorError::InvalidDocument { .. }
        ));
        unrelated.expect_running();
    }

    unrelated.stop_and_wait();
    unrelated_watch.expect_eof("the unrelated process exits only on request");
}
