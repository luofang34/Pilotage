use std::os::unix::fs::{MetadataExt, PermissionsExt};

use super::{TestResult, name, test_parent};
use crate::{
    DurabilityStep, DurableStore, FaultAction, FaultController, FaultRule, StorageError,
    StorageOperation,
};

#[test]
fn temporary_unlink_retry_rejects_changed_bytes() -> TestResult {
    let temporary = tempfile::Builder::new()
        .prefix("pilotage-unlink-rewrite-")
        .tempdir_in(test_parent()?)?;
    let root_path = temporary.path().join("store");
    let faults = deletion_faults();
    let store = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let temporary_name = name(".pilotage-tmp-1-000000000000000a")?;
    let temporary_path = root_path.join(temporary_name.as_os_str());
    std::fs::write(&temporary_path, b"partial")?;
    std::fs::set_permissions(&temporary_path, std::fs::Permissions::from_mode(0o600))?;
    let identity = std::fs::metadata(&temporary_path)?;
    let owned = root.inspect_owned_temporary(&temporary_name, 7)?;
    let rewrite_path = temporary_path.clone();
    faults.set_test_hook(move || {
        std::fs::write(&rewrite_path, b"changed").expect("rewrite exact removal target");
    })?;

    let error = lease
        .cleanup_owned_temporary(&root, &owned)
        .expect_err("retry must reject changed temporary bytes");
    assert!(matches!(error, StorageError::ContentMismatch { .. }));
    assert!(error.poisons_authorization());
    assert_eq!(std::fs::read(&temporary_path)?, b"changed");
    assert_eq!(std::fs::metadata(&temporary_path)?.ino(), identity.ino());
    assert!(faults.is_exhausted()?);
    Ok(())
}

#[test]
fn temporary_unlink_retry_revalidates_the_writer_lease() -> TestResult {
    let temporary = tempfile::Builder::new()
        .prefix("pilotage-unlink-lease-")
        .tempdir_in(test_parent()?)?;
    let root_path = temporary.path().join("store");
    let faults = deletion_faults();
    let store = DurableStore::open_or_create_with_faults(&root_path, faults.clone())?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let temporary_name = name(".pilotage-tmp-1-000000000000000b")?;
    let temporary_path = root_path.join(temporary_name.as_os_str());
    std::fs::write(&temporary_path, b"partial")?;
    std::fs::set_permissions(&temporary_path, std::fs::Permissions::from_mode(0o600))?;
    let owned = root.inspect_owned_temporary(&temporary_name, 7)?;
    let lock = root_path.join(".pilotage-writer-lock");
    let held = root_path.join("held-writer-lock");
    let replacement = lock.clone();
    let moved = held.clone();
    faults.set_test_hook(move || {
        std::fs::rename(&replacement, &moved).expect("move held writer lock");
        std::fs::write(&replacement, b"").expect("make replacement writer lock");
        std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o600))
            .expect("make replacement writer lock private");
    })?;

    let error = lease
        .cleanup_owned_temporary(&root, &owned)
        .expect_err("retry must reject a replaced writer lease name");
    assert!(matches!(error, StorageError::Corruption { .. }));
    assert!(error.poisons_authorization());
    assert_eq!(std::fs::read(&temporary_path)?, b"partial");
    assert!(lock.exists());
    assert!(held.exists());
    assert!(faults.is_exhausted()?);
    Ok(())
}

fn deletion_faults() -> FaultController {
    FaultController::new([FaultRule::once(
        StorageOperation::RemoveTemporary,
        DurabilityStep::Deletion,
        FaultAction::FailBefore,
    )])
}
