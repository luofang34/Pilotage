use super::{TestResult, fixture, name};
use crate::{ExactObject, StorageError};

#[test]
fn a_removed_writer_lock_is_poisoning_and_stops_mutation() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let lease = store.acquire_writer()?;
    let root = store.root_directory();
    std::fs::remove_file(root_path.join(".pilotage-writer-lock"))?;

    let error = lease
        .validate(&root)
        .expect_err("a removed writer lock must invalidate the lease");
    assert!(matches!(
        error,
        StorageError::IdentityChanged { actual: None, .. }
    ));
    assert!(error.poisons_authorization());
    assert_eq!(
        error.context().component.as_ref(),
        Some(&name(".pilotage-writer-lock")?)
    );

    let destination = name("must-not-exist")?;
    let mutation = root
        .put_immutable_no_replace(
            &lease,
            &destination,
            &ExactObject::from_bytes(b"blocked".to_vec()),
        )
        .expect_err("an invalid lease must stop a later mutation");
    assert!(mutation.poisons_authorization());
    assert!(!root_path.join(destination.as_os_str()).exists());
    Ok(())
}
