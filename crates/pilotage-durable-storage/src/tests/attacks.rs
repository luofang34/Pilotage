use std::os::unix::fs::{PermissionsExt, symlink};

use super::{TestResult, fixture, name, test_parent, tree_limits};
use crate::{
    DurabilityStep, DurableStore, ExactObject, ObjectName, StorageError, StorageOperation,
};

#[test]
fn root_symlink_and_symlinked_tmp_ancestor_are_rejected() -> TestResult {
    let temporary = tempfile::Builder::new()
        .prefix("pilotage-root-link-")
        .tempdir_in(test_parent()?)?;
    let target = temporary.path().join("target");
    std::fs::create_dir(&target)?;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))?;
    let root_link = temporary.path().join("root");
    symlink(&target, &root_link)?;
    assert!(DurableStore::open_or_create(&root_link).is_err());

    let real_ancestor = temporary.path().join("real");
    std::fs::create_dir(&real_ancestor)?;
    std::fs::set_permissions(&real_ancestor, std::fs::Permissions::from_mode(0o700))?;
    let alias_ancestor = temporary.path().join("alias");
    symlink(&real_ancestor, &alias_ancestor)?;
    let requested = alias_ancestor.join("store");
    let error = DurableStore::open_or_create(&requested)
        .err()
        .ok_or_else(|| std::io::Error::other("a symlinked ancestor was accepted"))?;
    assert_eq!(
        error.context().requested_root.as_deref(),
        Some(requested.as_path())
    );
    assert_eq!(error.context().component.as_ref(), Some(&name("alias")?));

    #[cfg(target_vendor = "apple")]
    {
        let through_tmp = std::path::Path::new("/tmp")
            .join(format!("pilotage-reject-symlink-{}", std::process::id()));
        assert!(DurableStore::open_or_create(&through_tmp).is_err());
    }
    Ok(())
}

#[test]
fn invalid_root_component_keeps_the_requested_root() -> TestResult {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let requested = std::path::PathBuf::from(OsString::from_vec(
        b"/private/tmp/pilotage-invalid-\0-root".to_vec(),
    ));
    let error = DurableStore::open_or_create(&requested)
        .err()
        .ok_or_else(|| std::io::Error::other("a NUL root component was accepted"))?;
    assert!(matches!(error, StorageError::InvalidObjectName { .. }));
    assert_eq!(
        error.context().requested_root.as_deref(),
        Some(requested.as_path())
    );
    Ok(())
}

#[test]
fn existing_public_root_and_child_are_not_repaired_in_place() -> TestResult {
    let temporary = tempfile::Builder::new()
        .prefix("pilotage-public-root-")
        .tempdir_in(test_parent()?)?;
    let public_root = temporary.path().join("store");
    std::fs::create_dir(&public_root)?;
    std::fs::set_permissions(&public_root, std::fs::Permissions::from_mode(0o755))?;
    let error = DurableStore::open_or_create(&public_root)
        .err()
        .ok_or_else(|| std::io::Error::other("a public root was accepted"))?;
    assert!(matches!(error, StorageError::WrongMode { .. }));
    assert_eq!(error.context().component.as_ref(), Some(&name("store")?));
    assert!(error.context().root.is_some());
    assert_eq!(
        std::fs::metadata(&public_root)?.permissions().mode() & 0o777,
        0o755
    );

    std::fs::set_permissions(&public_root, std::fs::Permissions::from_mode(0o700))?;
    let store = DurableStore::open_or_create(&public_root)?;
    let lease = store.acquire_writer()?;
    std::fs::create_dir(public_root.join("public-child"))?;
    std::fs::set_permissions(
        public_root.join("public-child"),
        std::fs::Permissions::from_mode(0o755),
    )?;
    assert!(matches!(
        store.root_directory().child(&lease, &name("public-child")?),
        Err(StorageError::WrongMode { .. })
    ));
    assert_eq!(
        std::fs::metadata(public_root.join("public-child"))?
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    Ok(())
}

#[test]
fn root_name_swap_is_detected_before_later_access() -> TestResult {
    let (_temporary, root, store) = fixture()?;
    let moved = root.with_extension("held");
    std::fs::rename(&root, &moved)?;
    std::fs::create_dir(&root)?;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
    let error = store
        .root_directory()
        .list()
        .expect_err("a replacement root must fail");
    assert!(matches!(error, StorageError::RootChanged { .. }));
    assert!(error.context().requested_root.as_deref() == Some(root.as_path()));
    assert_eq!(error.context().component.as_ref(), Some(&name("store")?));
    Ok(())
}

#[test]
fn writer_validation_rejects_a_replaced_lock_name() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let lock = root_path.join(".pilotage-writer-lock");
    std::fs::rename(&lock, root_path.join("held-writer-lock"))?;
    std::fs::write(&lock, b"")?;
    std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o600))?;

    let error = lease
        .validate(&root)
        .expect_err("a replacement lock name must invalidate the held lease");
    assert!(matches!(error, StorageError::Corruption { .. }));
    assert_eq!(
        error.context().component.as_ref(),
        Some(&name(".pilotage-writer-lock")?)
    );

    let second_store = DurableStore::open_or_create(&root_path)?;
    let _second_lease = second_store.acquire_writer()?;
    Ok(())
}

#[test]
fn intermediate_directory_swap_is_detected() -> TestResult {
    let (temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let child_name = name("child")?;
    let child = store.root_directory().child(&lease, &child_name)?;
    let held = root_path.join("held-child");
    std::fs::rename(root_path.join("child"), &held)?;
    let outside = temporary.path().join("outside");
    std::fs::create_dir(&outside)?;
    symlink(&outside, root_path.join("child"))?;
    let error = child
        .list()
        .expect_err("an intermediate symlink swap must fail");
    assert!(matches!(
        error,
        StorageError::IdentityChanged { .. } | StorageError::WrongType { .. }
    ));
    assert_eq!(error.context().component.as_ref(), Some(&child_name));
    Ok(())
}

#[test]
fn public_and_hard_linked_files_are_rejected() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let private_name = name("private")?;
    root.put_immutable_no_replace(
        &lease,
        &private_name,
        &ExactObject::from_bytes(b"bytes".to_vec()),
    )?;
    std::fs::hard_link(root_path.join("private"), root_path.join("alias"))?;
    assert!(matches!(
        root.read_exact(&private_name, 5),
        Err(StorageError::LinkedObject { .. })
    ));

    std::fs::write(root_path.join("public"), b"public")?;
    std::fs::set_permissions(
        root_path.join("public"),
        std::fs::Permissions::from_mode(0o644),
    )?;
    assert!(matches!(
        root.exists(&name("public")?),
        Err(StorageError::WrongMode { .. })
    ));
    Ok(())
}

#[test]
fn direct_object_read_does_not_follow_a_symlink() -> TestResult {
    let (temporary, root_path, store) = fixture()?;
    let outside = temporary.path().join("outside-object");
    std::fs::write(&outside, b"outside")?;
    symlink(&outside, root_path.join("linked-object"))?;
    assert!(matches!(
        store
            .root_directory()
            .read_exact(&name("linked-object")?, 7),
        Err(StorageError::WrongType { .. })
    ));
    assert_eq!(std::fs::read(outside)?, b"outside");
    Ok(())
}

#[test]
fn safe_temporary_cleanup_refuses_public_files() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let private_temp = name(".pilotage-tmp-1-0000000000000000")?;
    std::fs::write(root_path.join(private_temp.as_os_str()), b"partial")?;
    std::fs::set_permissions(
        root_path.join(private_temp.as_os_str()),
        std::fs::Permissions::from_mode(0o600),
    )?;
    let owned = root.inspect_owned_temporary(&private_temp, 7)?;
    lease.cleanup_owned_temporary(&root, &owned)?;
    assert!(!root_path.join(private_temp.as_os_str()).exists());

    let public_temp = ObjectName::new(".pilotage-tmp-1-0000000000000001")?;
    std::fs::write(root_path.join(public_temp.as_os_str()), b"partial")?;
    std::fs::set_permissions(
        root_path.join(public_temp.as_os_str()),
        std::fs::Permissions::from_mode(0o644),
    )?;
    assert!(matches!(
        root.inspect_owned_temporary(&public_temp, 7),
        Err(StorageError::WrongMode { .. })
    ));
    assert!(root_path.join(public_temp.as_os_str()).exists());
    Ok(())
}

#[test]
fn temporary_cleanup_rejects_a_new_hard_link() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let temporary_name = name(".pilotage-tmp-1-0000000000000002")?;
    let temporary_path = root_path.join(temporary_name.as_os_str());
    std::fs::write(&temporary_path, b"partial")?;
    std::fs::set_permissions(&temporary_path, std::fs::Permissions::from_mode(0o600))?;
    let owned = root.inspect_owned_temporary(&temporary_name, 7)?;
    let alias = root_path.join("temporary-alias");
    std::fs::hard_link(&temporary_path, &alias)?;
    assert!(matches!(
        lease.cleanup_owned_temporary(&root, &owned),
        Err(StorageError::LinkedObject { .. })
    ));
    assert!(temporary_path.exists());
    assert!(alias.exists());
    Ok(())
}

#[test]
fn temporary_cleanup_rejects_a_token_moved_to_another_root() -> TestResult {
    let temporary = tempfile::Builder::new()
        .prefix("pilotage-temp-root-owner-")
        .tempdir_in(test_parent()?)?;
    let first_path = temporary.path().join("first");
    let second_path = temporary.path().join("second");
    let first = DurableStore::open_or_create(&first_path)?;
    let second = DurableStore::open_or_create(&second_path)?;
    let first_lease = first.acquire_writer()?;
    let second_lease = second.acquire_writer()?;
    let temporary_name = name(".pilotage-tmp-1-0000000000000003")?;
    let source = first_path.join(temporary_name.as_os_str());
    let destination = second_path.join(temporary_name.as_os_str());
    std::fs::write(&source, b"partial")?;
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600))?;
    let owned = first
        .root_directory()
        .inspect_owned_temporary(&temporary_name, 7)?;
    std::fs::rename(&source, &destination)?;

    let error = second_lease
        .cleanup_owned_temporary(&second.root_directory(), &owned)
        .expect_err("a token from another root must not authorize deletion");
    assert!(matches!(error, StorageError::Corruption { .. }));
    assert!(destination.exists());
    drop(first_lease);
    Ok(())
}

#[test]
fn temporary_cleanup_rejects_a_token_moved_to_another_directory() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let first_name = name("first")?;
    let second_name = name("second")?;
    let first = root.child(&lease, &first_name)?;
    let second = root.child(&lease, &second_name)?;
    let temporary_name = name(".pilotage-tmp-1-0000000000000004")?;
    let source = root_path
        .join(first_name.as_os_str())
        .join(temporary_name.as_os_str());
    let destination = root_path
        .join(second_name.as_os_str())
        .join(temporary_name.as_os_str());
    std::fs::write(&source, b"partial")?;
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600))?;
    let owned = first.inspect_owned_temporary(&temporary_name, 7)?;
    std::fs::rename(&source, &destination)?;

    let error = lease
        .cleanup_owned_temporary(&second, &owned)
        .expect_err("a token from another directory must not authorize deletion");
    assert!(matches!(error, StorageError::IdentityChanged { .. }));
    assert!(destination.exists());
    Ok(())
}

#[test]
fn removal_postcondition_rejects_a_recreated_name_without_deleting_it() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let recreated = name("recreated")?;
    let recreated_path = root_path.join(recreated.as_os_str());
    std::fs::write(&recreated_path, b"replacement")?;
    std::fs::set_permissions(&recreated_path, std::fs::Permissions::from_mode(0o600))?;
    let context = root.handle.context(
        Some(&recreated),
        StorageOperation::RemoveTree,
        DurabilityStep::AfterMutation,
    );

    let error = crate::unix::validate_absent_after(&lease, &root, &recreated, context)
        .expect_err("a recreated name must invalidate removal success");
    assert!(matches!(error, StorageError::Corruption { .. }));
    assert_eq!(std::fs::read(recreated_path)?, b"replacement");
    Ok(())
}

#[test]
fn tree_preflight_makes_public_tree_a_zero_delete() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let tree_name = name("tree")?;
    let tree = root.child(&lease, &tree_name)?;
    for child in ["private", "public"] {
        tree.put_immutable_no_replace(
            &lease,
            &name(child)?,
            &ExactObject::from_bytes(child.as_bytes().to_vec()),
        )?;
    }
    std::fs::set_permissions(
        root_path.join("tree/public"),
        std::fs::Permissions::from_mode(0o644),
    )?;
    let public_directory = root_path.join("tree/public-directory");
    std::fs::create_dir(&public_directory)?;
    std::fs::set_permissions(&public_directory, std::fs::Permissions::from_mode(0o755))?;
    assert!(matches!(
        root.inspect_private_tree(&tree_name, tree_limits()),
        Err(StorageError::WrongMode { .. })
    ));
    assert!(root_path.join("tree/private").exists());
    assert!(root_path.join("tree/public").exists());
    assert!(public_directory.exists());
    Ok(())
}

#[test]
fn exact_private_tree_removal_does_not_follow_links() -> TestResult {
    let (temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let tree_name = name("tree")?;
    root.child(&lease, &tree_name)?;
    let outside = temporary.path().join("outside-file");
    std::fs::write(&outside, b"outside")?;
    symlink(&outside, root_path.join("tree/link"))?;
    assert!(
        root.inspect_private_tree(&tree_name, tree_limits())
            .is_err()
    );
    assert_eq!(std::fs::read(&outside)?, b"outside");
    assert!(root_path.join("tree/link").exists());
    Ok(())
}

#[test]
fn exact_private_tree_removal_deletes_only_the_selected_tree() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let tree_name = name("tree")?;
    let tree = root.child(&lease, &tree_name)?;
    let nested_name = name("nested")?;
    let nested = tree.child(&lease, &nested_name)?;
    nested.put_immutable_no_replace(
        &lease,
        &name("object")?,
        &ExactObject::from_bytes(b"object".to_vec()),
    )?;
    root.put_immutable_no_replace(
        &lease,
        &name("keep")?,
        &ExactObject::from_bytes(b"keep".to_vec()),
    )?;
    let manifest = root.inspect_private_tree(&tree_name, tree_limits())?;
    assert_eq!(manifest.object_count(), 3);
    lease.remove_private_tree(&root, &manifest)?;
    assert!(!root_path.join("tree").exists());
    assert_eq!(std::fs::read(root_path.join("keep"))?, b"keep");
    Ok(())
}

#[test]
fn exact_tree_removal_rejects_an_unlisted_private_descendant() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let tree_name = name("tree")?;
    let tree = root.child(&lease, &tree_name)?;
    let manifest = root.inspect_private_tree(&tree_name, tree_limits())?;
    tree.put_immutable_no_replace(
        &lease,
        &name("unknown")?,
        &ExactObject::from_bytes(b"unknown".to_vec()),
    )?;

    let error = lease
        .remove_private_tree(&root, &manifest)
        .expect_err("an object absent from the manifest must stop deletion");
    assert!(matches!(error, StorageError::Corruption { .. }));
    assert!(root_path.join("tree").exists());
    assert_eq!(std::fs::read(root_path.join("tree/unknown"))?, b"unknown");
    Ok(())
}

#[test]
fn exact_tree_removal_rejects_in_place_file_changes() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    let tree_name = name("tree")?;
    let tree = root.child(&lease, &tree_name)?;
    let file_name = name("object")?;
    tree.put_immutable_no_replace(
        &lease,
        &file_name,
        &ExactObject::from_bytes(b"original".to_vec()),
    )?;
    let manifest = root.inspect_private_tree(&tree_name, tree_limits())?;
    let file_path = root_path.join("tree/object");
    std::fs::write(&file_path, b"modified")?;

    let error = lease
        .remove_private_tree(&root, &manifest)
        .expect_err("changed file bytes must stop tree removal");
    assert!(matches!(error, StorageError::ContentMismatch { .. }));
    assert!(root_path.join("tree").exists());
    assert_eq!(std::fs::read(file_path)?, b"modified");
    Ok(())
}
