use pilotage_durable_storage::{
    DurabilityStep, FaultAction, FaultController, FaultRule, StorageError, StorageOperation,
};

use crate::AviateSupervisorError;
use crate::document::{
    ProcessIdentity, ProcessStartIdentity, SCHEMA_VERSION, TARGET_ATTESTATION_NAME,
    TargetAttestation,
};
use crate::lease_store::{LeaseStore, digest_bytes};
use crate::supervisor::owner::TargetPublicationState;

use super::resolve_target_publication;

#[test]
fn ambiguous_target_publication_resolves_to_its_exact_digest() {
    let temporary = test_root("target-linked-publication");
    let root = temporary.path().join("storage");
    let faults = FaultController::new([
        FaultRule::once(
            StorageOperation::PublishImmutable,
            DurabilityStep::ObjectPublication,
            FaultAction::LoseAckAfter,
        ),
        FaultRule::once(
            StorageOperation::PublishImmutable,
            DurabilityStep::ObjectReadback,
            FaultAction::FailBefore,
        ),
    ]);
    let store = LeaseStore::create_fresh_with_faults(&root, faults.clone())
        .expect("create faulted target store");
    let expected = target_attestation();
    let mut state = TargetPublicationState::Candidate(expected.clone());

    let error = store
        .publish(TARGET_ATTESTATION_NAME, &expected)
        .expect_err("lose the linked publication acknowledgement");
    assert!(matches!(
        error,
        AviateSupervisorError::Storage { source, .. }
            if matches!(source.as_ref(), StorageError::AmbiguousCommit { .. })
    ));
    let expected_bytes = serde_json::to_vec(&expected).expect("encode expected attestation");
    let expected_digest = digest_bytes(&expected_bytes);
    let resolved =
        resolve_target_publication(&store, &mut state).expect("repair target publication");

    assert_eq!(resolved, Some(expected_digest));
    assert_eq!(
        state,
        TargetPublicationState::Published {
            attestation: expected.clone(),
            digest: expected_digest,
        }
    );
    let (stored, stored_digest): (TargetAttestation, _) = store
        .read(TARGET_ATTESTATION_NAME)
        .expect("read repaired target attestation");
    assert_eq!(stored, expected);
    assert_eq!(stored_digest, expected_digest);
    assert_eq!(temporary_object_count(&root), 0);
    assert!(faults.is_exhausted().expect("inspect target faults"));
}

#[test]
fn uncommitted_target_candidate_resolves_to_absence() {
    let temporary = test_root("target-uncommitted-publication");
    let root = temporary.path().join("storage");
    let faults = FaultController::new([FaultRule::once(
        StorageOperation::PublishImmutable,
        DurabilityStep::ObjectData,
        FaultAction::FailBefore,
    )]);
    let store = LeaseStore::create_fresh_with_faults(&root, faults.clone())
        .expect("create faulted target store");
    let expected = target_attestation();
    let mut state = TargetPublicationState::Candidate(expected.clone());

    let error = store
        .publish(TARGET_ATTESTATION_NAME, &expected)
        .expect_err("stop target publication before commit");
    assert!(matches!(
        error,
        AviateSupervisorError::Storage { source, .. }
            if matches!(source.as_ref(), StorageError::InjectedFault { .. })
    ));
    let resolved =
        resolve_target_publication(&store, &mut state).expect("classify absent target publication");

    assert_eq!(resolved, None);
    assert_eq!(state, TargetPublicationState::NotAttempted);
    assert!(!root.join(TARGET_ATTESTATION_NAME).exists());
    assert!(faults.is_exhausted().expect("inspect target faults"));
}

fn target_attestation() -> TargetAttestation {
    let argument_digest = fixed_digest(3);
    TargetAttestation {
        schema_version: SCHEMA_VERSION,
        run_intent_digest: fixed_digest(1),
        target: ProcessIdentity {
            pid: 13,
            process_group: 11,
            session_id: 11,
            parent_pid: 11,
            real_user_id: 501,
            start: ProcessStartIdentity::Linux {
                boot_id: "12345678-1234-1234-1234-123456789abc".to_owned(),
                start_ticks: 17,
            },
            executable: "/private/target".into(),
            executable_digest: fixed_digest(2),
            launch_argv_digest: argument_digest,
            observed_argv_digest: Some(argument_digest),
        },
    }
}

fn fixed_digest(byte: u8) -> flight_tune::Digest {
    flight_tune::Digest::from_bytes([byte; 32])
}

fn test_root(prefix: &str) -> tempfile::TempDir {
    let temporary_parent = std::fs::canonicalize("/tmp").expect("canonical temporary parent");
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(temporary_parent)
        .expect("create target publication root")
}

fn temporary_object_count(root: &std::path::Path) -> usize {
    std::fs::read_dir(root)
        .expect("read target publication root")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".pilotage-tmp-")
        })
        .count()
}
