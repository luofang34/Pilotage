#![allow(clippy::expect_used, clippy::panic)]

mod attacks;
mod cas;
mod faults;
mod lease;
mod removal_recovery;

use std::error::Error;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use tempfile::TempDir;

use crate::{
    CasOutcome, CompareExchangeError, DurableStore, ExactObject, ExpectedValue, ObjectName,
    PrivateTreeLimits, PutOutcome, StorageError, digest_bytes,
};

type TestResult = Result<(), Box<dyn Error>>;

fn fixture() -> Result<(TempDir, PathBuf, DurableStore), Box<dyn Error>> {
    let temporary = tempfile::Builder::new()
        .prefix("pilotage-durable-")
        .tempdir_in(test_parent()?)?;
    let root = temporary.path().join("store");
    let store = DurableStore::open_or_create(&root)?;
    Ok((temporary, root, store))
}

fn test_parent() -> Result<PathBuf, Box<dyn Error>> {
    #[cfg(target_vendor = "apple")]
    {
        Ok(PathBuf::from("/private/tmp"))
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        Ok(std::env::temp_dir().canonicalize()?)
    }
}

fn name(value: &str) -> Result<ObjectName, StorageError> {
    ObjectName::new(value)
}

fn tree_limits() -> PrivateTreeLimits {
    PrivateTreeLimits::new(128, 1024 * 1024, 8 * 1024 * 1024)
}

#[test]
fn object_name_accepts_one_non_utf8_component() -> TestResult {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let name = ObjectName::new(OsStr::from_bytes(b"object-\xff"))?;
    assert_eq!(name.as_os_str().as_bytes(), b"object-\xff");
    for invalid in ["", ".", "..", "a/b", "/absolute"] {
        assert!(matches!(
            ObjectName::new(invalid),
            Err(StorageError::InvalidObjectName { .. })
        ));
    }
    Ok(())
}

#[test]
fn root_child_and_files_have_exact_private_modes() -> TestResult {
    let (_temporary, root, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let child_name = name("objects")?;
    let child = store.root_directory().child(&lease, &child_name)?;
    let object_name = name("one")?;
    let object = ExactObject::from_bytes(b"private".to_vec());
    assert_eq!(
        child.put_immutable_no_replace(&lease, &object_name, &object)?,
        PutOutcome::Published
    );
    assert_eq!(
        std::fs::metadata(&root)?.permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(root.join("objects"))?
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(root.join("objects/one"))?
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    Ok(())
}

#[test]
fn immutable_publication_is_exact_and_idempotent() -> TestResult {
    let (_temporary, _root, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let object_name = name("entry")?;
    let object = ExactObject::from_bytes(b"complete".to_vec());
    assert_eq!(
        root.put_immutable_no_replace(&lease, &object_name, &object)?,
        PutOutcome::Published
    );
    assert_eq!(root.read_exact(&object_name, 8)?, object);
    assert_eq!(
        root.put_immutable_no_replace(&lease, &object_name, &object)?,
        PutOutcome::AlreadyExact
    );
    let wrong = ExactObject::from_bytes(b"changed!".to_vec());
    let error = root
        .put_immutable_no_replace(&lease, &object_name, &wrong)
        .expect_err("different bytes must fail");
    assert!(matches!(error, StorageError::ContentMismatch { .. }));
    assert!(error.poisons_authorization());
    Ok(())
}

#[test]
fn mutable_and_immutable_destinations_reject_internal_names() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let object = ExactObject::from_bytes(b"object".to_vec());
    for value in [".pilotage-tmp-user", ".pilotage-writer-lock"] {
        let reserved = name(value)?;
        let immutable = root
            .put_immutable_no_replace(&lease, &reserved, &object)
            .expect_err("an internal immutable name must fail");
        assert!(matches!(immutable, StorageError::InvalidObjectName { .. }));
        assert_eq!(immutable.context().component.as_ref(), Some(&reserved));
        assert_eq!(immutable.context().object, Some(object.digest()));

        let mutable = lease
            .compare_exchange_file(&root, &reserved, ExpectedValue::Absent, object.clone())
            .expect_err("an internal mutable name must fail");
        assert!(matches!(mutable, StorageError::InvalidObjectName { .. }));
        assert_eq!(mutable.context().component.as_ref(), Some(&reserved));
        assert_eq!(mutable.context().object, Some(object.digest()));
    }
    assert!(!root_path.join(".pilotage-tmp-user").exists());
    Ok(())
}

#[test]
fn temporary_namespace_exhaustion_reports_the_selected_object() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    for counter in 0_u64..64 {
        let path = root_path.join(format!(
            ".pilotage-tmp-{}-{counter:016x}",
            std::process::id()
        ));
        std::fs::write(&path, b"")?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    let object = ExactObject::from_bytes(b"selected".to_vec());
    let error = store
        .root_directory()
        .put_immutable_no_replace(&lease, &name("destination")?, &object)
        .expect_err("an exhausted private namespace must fail");
    assert!(matches!(error, StorageError::Corruption { .. }));
    assert_eq!(error.context().object, Some(object.digest()));
    Ok(())
}

#[test]
fn read_limit_is_enforced_before_allocation() -> TestResult {
    let (_temporary, _root, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let object_name = name("bounded")?;
    root.put_immutable_no_replace(
        &lease,
        &object_name,
        &ExactObject::from_bytes(vec![7_u8; 64]),
    )?;
    assert!(matches!(
        root.read_exact(&object_name, 63),
        Err(StorageError::ObjectTooLarge { .. })
    ));
    Ok(())
}

#[test]
fn digest_bound_read_errors_keep_the_expected_digest() -> TestResult {
    let (_temporary, _root_path, store) = fixture()?;
    let root = store.root_directory();
    let missing = name("missing")?;
    let expected = digest_bytes(b"expected");
    let error = root
        .read_digest(&missing, expected, 8)
        .expect_err("a missing digest-bound object must fail");
    assert!(matches!(error, StorageError::Io { .. }));
    assert_eq!(error.context().component.as_ref(), Some(&missing));
    assert_eq!(error.context().object, Some(expected));
    Ok(())
}

#[test]
fn regular_read_handles_are_nonblocking() -> TestResult {
    let (_temporary, _root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let object_name = name("nonblocking")?;
    root.put_immutable_no_replace(
        &lease,
        &object_name,
        &ExactObject::from_bytes(b"object".to_vec()),
    )?;
    let flags = crate::unix::regular_open_flags_for_test(&root, &object_name)?;
    assert!(flags.contains(rustix::fs::OFlags::NONBLOCK));
    Ok(())
}

#[test]
fn private_tree_manifest_enforces_file_and_total_bounds() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let tree_name = name("tree")?;
    let tree = root.child(&lease, &tree_name)?;
    tree.put_immutable_no_replace(
        &lease,
        &name("object")?,
        &ExactObject::from_bytes(b"eight123".to_vec()),
    )?;

    for limits in [
        PrivateTreeLimits::new(2, 7, 64),
        PrivateTreeLimits::new(2, 64, 7),
    ] {
        assert!(matches!(
            root.inspect_private_tree(&tree_name, limits),
            Err(StorageError::ObjectTooLarge { .. })
        ));
    }
    assert_eq!(std::fs::read(root_path.join("tree/object"))?, b"eight123");
    Ok(())
}

#[test]
fn one_writer_lease_excludes_another_open_session() -> TestResult {
    let (_temporary, root, store) = fixture()?;
    let first = store.acquire_writer()?;
    let second_store = DurableStore::open_or_create(&root)?;
    let error = second_store
        .acquire_writer()
        .err()
        .ok_or_else(|| std::io::Error::other("second writer unexpectedly acquired the lock"))?;
    assert!(error.is_writer_locked());
    drop(first);
    second_store.acquire_writer()?;
    Ok(())
}

#[test]
fn compare_exchange_rejects_stale_and_sibling_values() -> TestResult {
    let (_temporary, _root, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let head = name("HEAD")?;
    let first = ExactObject::from_bytes(b"first".to_vec());
    let second = ExactObject::from_bytes(b"second".to_vec());
    let sibling = ExactObject::from_bytes(b"sibling".to_vec());
    assert_eq!(
        lease.compare_exchange_file(&root, &head, ExpectedValue::Absent, first.clone(),)?,
        CasOutcome::Exchanged
    );
    assert_eq!(
        lease.compare_exchange_file(
            &root,
            &head,
            ExpectedValue::Exact(first.clone()),
            second.clone(),
        )?,
        CasOutcome::Exchanged
    );
    let stale = lease
        .compare_exchange_file(&root, &head, ExpectedValue::Exact(first), sibling)
        .expect_err("a stale writer must not replace a sibling");
    assert!(matches!(stale, StorageError::StaleExpected { .. }));
    assert!(stale.poisons_authorization());
    assert_eq!(root.read_exact(&head, 6)?, second);
    Ok(())
}

#[test]
fn exact_new_value_is_an_idempotent_cas_success() -> TestResult {
    let (_temporary, _root, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let head = name("HEAD")?;
    let value = ExactObject::from_bytes(b"value".to_vec());
    lease.compare_exchange_file(&root, &head, ExpectedValue::Absent, value.clone())?;
    assert_eq!(
        lease.compare_exchange_file(&root, &head, ExpectedValue::Absent, value)?,
        CasOutcome::AlreadyExact
    );
    Ok(())
}

#[test]
fn guarded_compare_exchange_cleans_up_after_validation_rejection() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let head = name("HEAD")?;
    let new = ExactObject::from_bytes(b"new".to_vec());
    let error = lease
        .compare_exchange_file_guarded(&root, &head, ExpectedValue::Absent, new, || {
            Err(std::io::Error::other("validation rejected"))
        })
        .expect_err("caller validation must stop authorization");
    assert!(matches!(error, CompareExchangeError::Validation { .. }));
    assert!(!root.exists(&head)?);
    assert!(std::fs::read_dir(root_path)?.all(|entry| {
        entry.is_ok_and(|entry| {
            !entry
                .file_name()
                .to_string_lossy()
                .starts_with(".pilotage-tmp-")
        })
    }));
    Ok(())
}

#[test]
fn guarded_compare_exchange_rechecks_a_sibling_after_validation() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let head = name("HEAD")?;
    let old = ExactObject::from_bytes(b"old".to_vec());
    let new = ExactObject::from_bytes(b"new".to_vec());
    let sibling = ExactObject::from_bytes(b"sib".to_vec());
    lease.compare_exchange_file(&root, &head, ExpectedValue::Absent, old.clone())?;

    let sibling_path = root_path.join("sibling");
    let head_path = root_path.join(head.as_os_str());
    let error = lease
        .compare_exchange_file_guarded(&root, &head, ExpectedValue::Exact(old), new, || {
            std::fs::write(&sibling_path, sibling.bytes())?;
            std::fs::set_permissions(&sibling_path, std::fs::Permissions::from_mode(0o600))?;
            std::fs::rename(&sibling_path, &head_path)
        })
        .expect_err("a sibling created by validation must not be replaced");
    assert!(matches!(
        error,
        CompareExchangeError::Storage {
            source: StorageError::StaleExpected { .. }
        }
    ));
    assert_eq!(root.read_exact(&head, sibling.bytes().len())?, sibling);
    Ok(())
}

#[test]
fn guarded_compare_exchange_rechecks_the_temporary_after_validation() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let head = name("HEAD")?;
    let new = ExactObject::from_bytes(b"new".to_vec());
    let error = lease
        .compare_exchange_file_guarded(&root, &head, ExpectedValue::Absent, new, || {
            let temporary = std::fs::read_dir(&root_path)?
                .filter_map(Result::ok)
                .find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".pilotage-tmp-")
                })
                .ok_or_else(|| std::io::Error::other("no CAS temporary"))?;
            std::fs::write(temporary.path(), b"bad")
        })
        .expect_err("a changed temporary must not be authorized");
    assert!(matches!(
        error,
        CompareExchangeError::Storage {
            source: StorageError::ContentMismatch { .. }
        }
    ));
    assert!(!root.exists(&head)?);
    Ok(())
}

#[test]
fn sequential_writer_cannot_replace_a_sibling_head() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let head = name("HEAD")?;
    let parent = ExactObject::from_bytes(b"parent".to_vec());
    let winner = ExactObject::from_bytes(b"winner".to_vec());
    let sibling = ExactObject::from_bytes(b"loser!".to_vec());
    let first_lease = store.acquire_writer()?;
    first_lease.compare_exchange_file(
        &store.root_directory(),
        &head,
        ExpectedValue::Absent,
        parent.clone(),
    )?;
    drop(first_lease);

    let winner_store = DurableStore::open_or_create(&root_path)?;
    let winner_lease = winner_store.acquire_writer()?;
    winner_lease.compare_exchange_file(
        &winner_store.root_directory(),
        &head,
        ExpectedValue::Exact(parent.clone()),
        winner.clone(),
    )?;
    drop(winner_lease);

    let stale_store = DurableStore::open_or_create(&root_path)?;
    let stale_lease = stale_store.acquire_writer()?;
    let error = stale_lease
        .compare_exchange_file(
            &stale_store.root_directory(),
            &head,
            ExpectedValue::Exact(parent),
            sibling,
        )
        .expect_err("a later writer must not replace a sibling");
    assert!(matches!(error, StorageError::StaleExpected { .. }));
    assert_eq!(stale_store.root_directory().read_exact(&head, 6)?, winner);
    Ok(())
}
