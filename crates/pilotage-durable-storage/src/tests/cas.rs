use std::os::unix::fs::PermissionsExt;

use super::{TestResult, fixture, name};

#[test]
fn absent_compare_exchange_never_replaces_an_existing_name() -> TestResult {
    let (_temporary, root_path, store) = fixture()?;
    let _lease = store.acquire_writer()?;
    let root = store.root_directory();
    let source = name("prepared")?;
    let destination = name("existing")?;
    let source_path = root_path.join(source.as_os_str());
    let destination_path = root_path.join(destination.as_os_str());
    std::fs::write(&source_path, b"new")?;
    std::fs::set_permissions(&source_path, std::fs::Permissions::from_mode(0o600))?;
    std::fs::write(&destination_path, b"old")?;
    std::fs::set_permissions(&destination_path, std::fs::Permissions::from_mode(0o600))?;

    let error = crate::unix::rename_absent_for_test(&root, &source, &destination)
        .expect_err("NOREPLACE must reject an existing destination");
    assert_eq!(error, rustix::io::Errno::EXIST);
    assert_eq!(std::fs::read(source_path)?, b"new");
    assert_eq!(std::fs::read(destination_path)?, b"old");
    Ok(())
}
