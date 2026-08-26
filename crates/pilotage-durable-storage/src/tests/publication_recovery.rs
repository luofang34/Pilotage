use super::{TestResult, name, test_parent};
use crate::{
    DurabilityStep, DurableStore, ExactObject, FaultAction, FaultController, FaultRule, ObjectName,
    StorageError, StorageOperation,
};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[test]
fn recovery_apis_reject_a_cross_root_lease_without_mutation() -> TestResult {
    let temporary = tempfile::Builder::new()
        .prefix("pilotage-repair-cross-root-")
        .tempdir_in(test_parent()?)?;
    let lease_root = temporary.path().join("lease-store");
    let empty_root = temporary.path().join("empty-store");
    let lease_store = DurableStore::open_or_create(&lease_root)?;
    let lease = lease_store.acquire_writer()?;
    let empty_store = DurableStore::open_or_create(&empty_root)?;
    let empty_directory = empty_store.root_directory();
    let before = store_entry_paths(&empty_root)?;
    assert!(before.is_empty());

    let object_name = name("absent")?;
    let repair_error = empty_directory
        .repair_immutable_publication_blocking(&lease, &object_name, 1)
        .expect_err("a cross-root lease must not repair an absent publication");
    assert!(matches!(
        repair_error,
        StorageError::Corruption {
            reason: "writer lease belongs to a different storage root",
            ..
        }
    ));
    assert_eq!(store_entry_paths(&empty_root)?, before);

    let cleanup_error = empty_directory
        .cleanup_unlinked_temporaries_blocking(&lease, 1, 1)
        .expect_err("a cross-root lease must not scan an empty store");
    assert!(matches!(
        cleanup_error,
        StorageError::Corruption {
            reason: "writer lease belongs to a different storage root",
            ..
        }
    ));
    assert_eq!(store_entry_paths(&empty_root)?, before);
    Ok(())
}

#[test]
fn writer_held_repair_recovers_linked_publication_after_reopen() -> TestResult {
    let temporary = tempfile::Builder::new()
        .prefix("pilotage-repair-publication-")
        .tempdir_in(test_parent()?)?;
    let root_path = temporary.path().join("store");
    let object_name = name("object")?;
    let expected = ExactObject::from_bytes(b"durable publication".to_vec());
    leave_linked_publication(&root_path, &object_name, &expected)?;

    let linked_temporary = one_temporary_path(&root_path)?;
    let destination = root_path.join(object_name.as_os_str());
    let destination_metadata = std::fs::metadata(&destination)?;
    let temporary_metadata = std::fs::metadata(&linked_temporary)?;
    assert_eq!(destination_metadata.dev(), temporary_metadata.dev());
    assert_eq!(destination_metadata.ino(), temporary_metadata.ino());
    assert_eq!(destination_metadata.nlink(), 2);
    assert_eq!(temporary_metadata.nlink(), 2);

    let reopened = DurableStore::open_existing_blocking(&root_path)?;
    let lease = reopened.acquire_writer()?;
    let root = reopened.root_directory();
    let repaired = root
        .repair_immutable_publication_blocking(&lease, &object_name, expected.bytes().len())?
        .ok_or_else(|| std::io::Error::other("the committed object was not found"))?;

    assert_eq!(repaired.bytes(), expected.bytes());
    assert_eq!(repaired.digest(), expected.digest());
    assert_eq!(std::fs::metadata(destination)?.nlink(), 1);
    assert!(!linked_temporary.exists());
    Ok(())
}

#[test]
fn unlinked_temporary_cleanup_respects_the_byte_limit() -> TestResult {
    let temporary = tempfile::Builder::new()
        .prefix("pilotage-repair-temporary-")
        .tempdir_in(test_parent()?)?;
    let root_path = temporary.path().join("store");
    let bytes = b"uncommitted bytes";
    leave_unlinked_temporary(&root_path, &name("object")?, bytes)?;
    let temporary_path = one_temporary_path(&root_path)?;
    assert_eq!(std::fs::metadata(&temporary_path)?.nlink(), 1);

    let reopened = DurableStore::open_existing_blocking(&root_path)?;
    let lease = reopened.acquire_writer()?;
    let root = reopened.root_directory();
    let error = root
        .cleanup_unlinked_temporaries_blocking(&lease, 1, bytes.len() - 1)
        .expect_err("an undersized byte limit must stop cleanup");
    assert!(matches!(
        error,
        StorageError::ObjectTooLarge {
            limit,
            actual,
            ..
        } if limit == bytes.len() - 1 && actual == bytes.len() as u64
    ));
    assert!(temporary_path.exists());

    assert_eq!(
        root.cleanup_unlinked_temporaries_blocking(&lease, 1, bytes.len())?,
        1
    );
    assert!(!temporary_path.exists());
    Ok(())
}

#[test]
fn unlinked_temporary_cleanup_respects_the_object_limit() -> TestResult {
    let temporary = tempfile::Builder::new()
        .prefix("pilotage-repair-temporary-count-")
        .tempdir_in(test_parent()?)?;
    let root_path = temporary.path().join("store");
    leave_unlinked_temporary(&root_path, &name("first")?, b"first")?;
    leave_unlinked_temporary(&root_path, &name("second")?, b"second")?;
    let temporary_paths = temporary_paths(&root_path)?;
    assert_eq!(temporary_paths.len(), 2);

    let reopened = DurableStore::open_existing_blocking(&root_path)?;
    let lease = reopened.acquire_writer()?;
    let root = reopened.root_directory();
    let error = root
        .cleanup_unlinked_temporaries_blocking(&lease, 1, 6)
        .expect_err("an undersized object limit must stop cleanup");
    assert!(matches!(
        error,
        StorageError::Corruption {
            reason: "the temporary object count exceeds the recovery limit",
            ..
        }
    ));
    assert!(temporary_paths.iter().all(|path| path.exists()));

    assert_eq!(root.cleanup_unlinked_temporaries_blocking(&lease, 2, 6)?, 2);
    assert!(temporary_paths.iter().all(|path| !path.exists()));
    Ok(())
}

fn leave_linked_publication(
    root_path: &Path,
    object_name: &ObjectName,
    object: &ExactObject,
) -> TestResult {
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
    let store = DurableStore::open_or_create_with_faults(root_path, faults.clone())?;
    let lease = store.acquire_writer()?;
    let error = store
        .root_directory()
        .put_immutable_no_replace(&lease, object_name, object)
        .expect_err("the readback fault must leave an ambiguous publication");
    assert!(matches!(error, StorageError::AmbiguousCommit { .. }));
    assert!(faults.is_exhausted()?);
    Ok(())
}

fn leave_unlinked_temporary(
    root_path: &Path,
    object_name: &ObjectName,
    bytes: &[u8],
) -> TestResult {
    let faults = FaultController::new([FaultRule::once(
        StorageOperation::PublishImmutable,
        DurabilityStep::ObjectData,
        FaultAction::FailBefore,
    )]);
    let store = DurableStore::open_or_create_with_faults(root_path, faults.clone())?;
    let lease = store.acquire_writer()?;
    let error = store
        .root_directory()
        .put_immutable_no_replace(
            &lease,
            object_name,
            &ExactObject::from_bytes(bytes.to_vec()),
        )
        .expect_err("the data fault must leave an unlinked temporary object");
    assert!(matches!(error, StorageError::InjectedFault { .. }));
    assert!(faults.is_exhausted()?);
    Ok(())
}

fn one_temporary_path(root_path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut paths = temporary_paths(root_path)?;
    if paths.len() != 1 {
        return Err(std::io::Error::other("the store does not contain one temporary").into());
    }
    paths
        .pop()
        .ok_or_else(|| std::io::Error::other("the temporary object disappeared").into())
}

fn temporary_paths(root_path: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(root_path)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(".pilotage-tmp-")
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn store_entry_paths(root_path: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut paths = std::fs::read_dir(root_path)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    Ok(paths)
}
