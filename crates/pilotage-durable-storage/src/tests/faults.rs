use super::{TestResult, fixture, name, test_parent, tree_limits};
use crate::{
    CasOutcome, DurabilityStep, DurableStore, ExactObject, ExpectedValue, FaultAction,
    FaultController, FaultRule, PutOutcome, StorageError, StorageOperation,
};
use std::os::unix::fs::MetadataExt;

#[test]
fn root_parent_sync_failure_is_repaired_by_reopen() -> TestResult {
    let temporary = tempfile::Builder::new()
        .prefix("pilotage-root-fault-")
        .tempdir_in(test_parent()?)?;
    let root_path = temporary.path().join("store");
    let faults = FaultController::new([FaultRule::once(
        StorageOperation::OpenRoot,
        DurabilityStep::ParentDirectory,
        FaultAction::FailBefore,
    )]);
    let error = DurableStore::open_or_create_with_faults(&root_path, faults.clone())
        .err()
        .ok_or_else(|| std::io::Error::other("a root barrier fault was ignored"))?;
    assert!(matches!(error, StorageError::InjectedFault { .. }));
    assert_eq!(
        error.context().requested_root.as_deref(),
        Some(root_path.as_path())
    );
    assert_eq!(error.context().component.as_ref(), Some(&name("store")?));
    assert!(error.context().root.is_some());
    assert!(faults.is_exhausted()?);
    let reopened = DurableStore::open_or_create(&root_path)?;
    reopened.acquire_writer()?;
    Ok(())
}

#[test]
fn lost_creation_ack_reopens_and_stabilizes_exact_directories() -> TestResult {
    let temporary = tempfile::Builder::new()
        .prefix("pilotage-create-ack-")
        .tempdir_in(test_parent()?)?;
    let root_path = temporary.path().join("store");
    let root_faults = FaultController::new([FaultRule::once(
        StorageOperation::OpenRoot,
        DurabilityStep::Creation,
        FaultAction::LoseAckAfter,
    )]);
    let store = DurableStore::open_or_create_with_faults(&root_path, root_faults.clone())?;
    assert!(root_faults.is_exhausted()?);

    let child_faults = FaultController::new([FaultRule::once(
        StorageOperation::CreateDirectory,
        DurabilityStep::Creation,
        FaultAction::LoseAckAfter,
    )]);
    let faulted = DurableStore::open_or_create_with_faults(&root_path, child_faults.clone())?;
    let lease = faulted.acquire_writer()?;
    faulted.root_directory().child(&lease, &name("child")?)?;
    assert!(child_faults.is_exhausted()?);
    drop(store);
    Ok(())
}

#[test]
fn writer_lock_parent_sync_failure_is_repaired_by_retry() -> TestResult {
    let (_temporary, root_path, _store) = fixture()?;
    let faults = FaultController::new([FaultRule::once(
        StorageOperation::AcquireWriter,
        DurabilityStep::ParentDirectory,
        FaultAction::FailBefore,
    )]);
    let store = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    assert!(store.acquire_writer().is_err());
    assert!(faults.is_exhausted()?);
    store.acquire_writer()?;
    Ok(())
}

#[test]
fn directory_parent_sync_failure_is_repaired_by_retry() -> TestResult {
    let temporary = tempfile::Builder::new()
        .prefix("pilotage-dir-fault-")
        .tempdir_in(test_parent()?)?;
    let root_path = temporary.path().join("store");
    let _store = DurableStore::open_or_create(&root_path)?;
    let faults = FaultController::new([FaultRule::once(
        StorageOperation::CreateDirectory,
        DurabilityStep::ParentDirectory,
        FaultAction::FailBefore,
    )]);
    let faulted = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    let lease = faulted.acquire_writer()?;
    let child_name = name("child")?;
    assert!(faulted.root_directory().child(&lease, &child_name).is_err());
    assert!(faults.is_exhausted()?);
    drop(lease);
    let retry = DurableStore::open_or_create(&root_path)?;
    let retry_lease = retry.acquire_writer()?;
    retry.root_directory().child(&retry_lease, &child_name)?;
    Ok(())
}

#[test]
fn immutable_lost_link_ack_recovers_exact_object() -> TestResult {
    let (_temporary, root_path, _store) = fixture()?;
    let faults = FaultController::new([FaultRule::once(
        StorageOperation::PublishImmutable,
        DurabilityStep::ObjectPublication,
        FaultAction::LoseAckAfter,
    )]);
    let store = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let object_name = name("object")?;
    let object = ExactObject::from_bytes(b"durable".to_vec());
    assert_eq!(
        root.put_immutable_no_replace(&lease, &object_name, &object)?,
        PutOutcome::Published
    );
    assert_eq!(root.read_exact(&object_name, 7)?, object);
    assert!(faults.is_exhausted()?);
    assert!(root.list()?.iter().all(|name| {
        !name
            .as_os_str()
            .to_string_lossy()
            .starts_with(".pilotage-tmp-")
    }));
    Ok(())
}

#[test]
fn immutable_crash_state_is_repaired_after_reopen() -> TestResult {
    let temporary = tempfile::Builder::new()
        .prefix("pilotage-publish-crash-")
        .tempdir_in(test_parent()?)?;
    let root_path = temporary.path().join("store");
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
    let store = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let object_name = name("object")?;
    let object = ExactObject::from_bytes(b"durable".to_vec());
    let error = root
        .put_immutable_no_replace(&lease, &object_name, &object)
        .expect_err("readback failure must stop linked publication recovery");
    assert!(matches!(error, StorageError::AmbiguousCommit { .. }));
    assert!(error.poisons_authorization());
    assert!(faults.is_exhausted()?);

    let linked_temporary = find_one_temporary(&root_path)?;
    let destination_metadata = std::fs::metadata(root_path.join("object"))?;
    let temporary_metadata = std::fs::metadata(&linked_temporary)?;
    assert_eq!(destination_metadata.dev(), temporary_metadata.dev());
    assert_eq!(destination_metadata.ino(), temporary_metadata.ino());
    assert_eq!(destination_metadata.nlink(), 2);
    assert_eq!(temporary_metadata.nlink(), 2);
    assert!(matches!(
        root.read_exact(&object_name, 7),
        Err(StorageError::LinkedObject { .. })
    ));
    drop(root);
    drop(lease);
    drop(store);

    let reopened = DurableStore::open_or_create(&root_path)?;
    let reopened_lease = reopened.acquire_writer()?;
    let reopened_root = reopened.root_directory();
    assert_eq!(
        reopened_root.put_immutable_no_replace(&reopened_lease, &object_name, &object)?,
        PutOutcome::AlreadyExact
    );
    assert_eq!(reopened_root.read_exact(&object_name, 7)?, object);
    assert_eq!(std::fs::metadata(root_path.join("object"))?.nlink(), 1);
    assert!(!linked_temporary.exists());
    Ok(())
}

fn find_one_temporary(
    root: &std::path::Path,
) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let mut found = None;
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(".pilotage-tmp-")
        {
            continue;
        }
        if found.replace(entry.path()).is_some() {
            return Err(std::io::Error::other("more than one temporary remained").into());
        }
    }
    found.ok_or_else(|| std::io::Error::other("no linked temporary remained").into())
}

#[test]
fn immutable_parent_sync_failure_uses_a_recovery_barrier() -> TestResult {
    let (_temporary, root_path, _store) = fixture()?;
    let faults = FaultController::new([FaultRule::once(
        StorageOperation::PublishImmutable,
        DurabilityStep::ParentDirectory,
        FaultAction::FailBefore,
    )]);
    let store = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    let lease = store.acquire_writer()?;
    let object_name = name("object")?;
    let object = ExactObject::from_bytes(b"durable".to_vec());
    assert_eq!(
        store
            .root_directory()
            .put_immutable_no_replace(&lease, &object_name, &object,)?,
        PutOutcome::Published
    );
    assert_eq!(store.root_directory().read_exact(&object_name, 7)?, object);
    assert!(faults.is_exhausted()?);
    Ok(())
}

#[test]
fn lost_temporary_unlink_ack_is_resolved_without_deleting_another_name() -> TestResult {
    let (_temporary, root_path, _store) = fixture()?;
    let faults = FaultController::new([FaultRule::once(
        StorageOperation::PublishImmutable,
        DurabilityStep::Deletion,
        FaultAction::LoseAckAfter,
    )]);
    let store = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    let lease = store.acquire_writer()?;
    let object_name = name("object")?;
    let object = ExactObject::from_bytes(b"durable".to_vec());
    store
        .root_directory()
        .put_immutable_no_replace(&lease, &object_name, &object)?;
    assert_eq!(store.root_directory().read_exact(&object_name, 7)?, object);
    assert!(store.root_directory().list()?.iter().all(|entry| {
        !entry
            .as_os_str()
            .to_string_lossy()
            .starts_with(".pilotage-tmp-")
    }));
    assert!(faults.is_exhausted()?);
    Ok(())
}

#[test]
fn lost_tree_unlink_ack_is_resolved_at_the_parent() -> TestResult {
    let (_temporary, root_path, setup) = fixture()?;
    let setup_lease = setup.acquire_writer()?;
    let tree_name = name("tree")?;
    setup.root_directory().child(&setup_lease, &tree_name)?;
    drop(setup_lease);

    let faults = FaultController::new([FaultRule::once(
        StorageOperation::RemoveTree,
        DurabilityStep::Deletion,
        FaultAction::LoseAckAfter,
    )]);
    let store = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let manifest = root.inspect_private_tree(&tree_name, tree_limits())?;
    lease.remove_private_tree(&root, &manifest)?;
    assert!(!root.exists(&tree_name)?);
    assert!(faults.is_exhausted()?);
    Ok(())
}

#[test]
fn existing_immutable_object_gets_new_durability_barriers() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let object_name = name("object")?;
    let object = ExactObject::from_bytes(b"durable".to_vec());
    store
        .root_directory()
        .put_immutable_no_replace(&lease, &object_name, &object)?;
    drop(lease);

    let faults = FaultController::new([FaultRule::once(
        StorageOperation::PublishImmutable,
        DurabilityStep::ParentDirectory,
        FaultAction::FailBefore,
    )]);
    let reopened = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    let reopened_lease = reopened.acquire_writer()?;
    assert!(
        reopened
            .root_directory()
            .put_immutable_no_replace(&reopened_lease, &object_name, &object)
            .is_err()
    );
    assert!(faults.is_exhausted()?);
    Ok(())
}

#[test]
fn cas_parent_sync_failure_recovers_by_exact_readback() -> TestResult {
    let (_temporary, root_path, _store) = fixture()?;
    let faults = FaultController::new([FaultRule::once(
        StorageOperation::CompareExchange,
        DurabilityStep::ParentDirectory,
        FaultAction::FailBefore,
    )]);
    let store = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    let lease = store.acquire_writer()?;
    let head = name("HEAD")?;
    let value = ExactObject::from_bytes(b"new".to_vec());
    assert_eq!(
        lease.compare_exchange_file(
            &store.root_directory(),
            &head,
            ExpectedValue::Absent,
            value.clone(),
        )?,
        CasOutcome::AlreadyExact
    );
    assert_eq!(store.root_directory().read_exact(&head, 3)?, value);
    assert!(faults.is_exhausted()?);
    Ok(())
}

#[test]
fn cas_lost_rename_ack_recovers_exact_new_value() -> TestResult {
    let (_temporary, root_path, _store) = fixture()?;
    let faults = FaultController::new([FaultRule::once(
        StorageOperation::CompareExchange,
        DurabilityStep::AuthorizationRename,
        FaultAction::LoseAckAfter,
    )]);
    let store = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    let lease = store.acquire_writer()?;
    let head = name("HEAD")?;
    let value = ExactObject::from_bytes(b"new".to_vec());
    assert_eq!(
        lease.compare_exchange_file(
            &store.root_directory(),
            &head,
            ExpectedValue::Absent,
            value.clone(),
        )?,
        CasOutcome::AlreadyExact
    );
    assert_eq!(store.root_directory().read_exact(&head, 3)?, value);
    assert!(faults.is_exhausted()?);
    Ok(())
}

#[test]
fn cas_post_commit_readback_failure_is_ambiguous_and_poisoning() -> TestResult {
    let (_temporary, root_path, _store) = fixture()?;
    let faults = FaultController::new([FaultRule::once(
        StorageOperation::CompareExchange,
        DurabilityStep::AuthorizationReadback,
        FaultAction::FailBefore,
    )]);
    let store = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let head = name("HEAD")?;
    let new = ExactObject::from_bytes(b"new".to_vec());
    let error = lease
        .compare_exchange_file(&root, &head, ExpectedValue::Absent, new.clone())
        .expect_err("a failed post-commit readback must be ambiguous");
    assert!(matches!(error, StorageError::AmbiguousCommit { .. }));
    assert!(error.poisons_authorization());
    assert!(faults.is_exhausted()?);
    assert_eq!(root.read_exact(&head, new.bytes().len())?, new);
    drop(lease);

    let reopened = DurableStore::open_or_create(&root_path)?;
    let reopened_lease = reopened.acquire_writer()?;
    assert_eq!(
        reopened_lease.compare_exchange_file(
            &reopened.root_directory(),
            &head,
            ExpectedValue::Absent,
            new,
        )?,
        CasOutcome::AlreadyExact
    );
    Ok(())
}

#[test]
fn already_exact_barrier_failure_is_ambiguous() -> TestResult {
    for step in [DurabilityStep::ObjectData, DurabilityStep::ParentDirectory] {
        let (_temporary, root_path, store) = fixture()?;
        let head = name("HEAD")?;
        let value = ExactObject::from_bytes(b"new".to_vec());
        let setup_lease = store.acquire_writer()?;
        setup_lease.compare_exchange_file(
            &store.root_directory(),
            &head,
            ExpectedValue::Absent,
            value.clone(),
        )?;
        drop(setup_lease);

        let faults = FaultController::new([FaultRule::once(
            StorageOperation::CompareExchange,
            step,
            FaultAction::FailBefore,
        )]);
        let reopened = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
        let lease = reopened.acquire_writer()?;
        let error = lease
            .compare_exchange_file(
                &reopened.root_directory(),
                &head,
                ExpectedValue::Absent,
                value.clone(),
            )
            .expect_err("an exact-new barrier failure must be ambiguous");
        assert!(matches!(error, StorageError::AmbiguousCommit { .. }));
        assert!(error.poisons_authorization());
        assert!(faults.is_exhausted()?);
    }
    Ok(())
}

#[test]
fn cas_second_barrier_failure_is_ambiguous_and_poisoning() -> TestResult {
    let (_temporary, root_path, _store) = fixture()?;
    let faults = FaultController::new([
        FaultRule::once(
            StorageOperation::CompareExchange,
            DurabilityStep::ParentDirectory,
            FaultAction::FailBefore,
        ),
        FaultRule::once(
            StorageOperation::CompareExchange,
            DurabilityStep::RecoveryBarrier,
            FaultAction::FailBefore,
        ),
    ]);
    let store = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    let lease = store.acquire_writer()?;
    let error = lease
        .compare_exchange_file(
            &store.root_directory(),
            &name("HEAD")?,
            ExpectedValue::Absent,
            ExactObject::from_bytes(b"new".to_vec()),
        )
        .expect_err("two failed barriers must be ambiguous");
    assert!(matches!(error, StorageError::AmbiguousCommit { .. }));
    assert!(error.poisons_authorization());
    assert!(faults.is_exhausted()?);
    Ok(())
}

#[test]
fn cas_fail_before_rename_keeps_old_value_and_cleans_temp() -> TestResult {
    let (_temporary, root_path, _store) = fixture()?;
    let faults = FaultController::new([FaultRule::once(
        StorageOperation::CompareExchange,
        DurabilityStep::AuthorizationRename,
        FaultAction::FailBefore,
    )]);
    let store = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let head = name("HEAD")?;
    assert!(
        lease
            .compare_exchange_file(
                &root,
                &head,
                ExpectedValue::Absent,
                ExactObject::from_bytes(b"new".to_vec()),
            )
            .is_err()
    );
    assert!(!root.exists(&head)?);
    assert!(root.list()?.iter().all(|name| {
        !name
            .as_os_str()
            .to_string_lossy()
            .starts_with(".pilotage-tmp-")
    }));
    assert!(faults.is_exhausted()?);
    Ok(())
}
