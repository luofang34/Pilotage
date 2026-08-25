use std::os::unix::process::CommandExt as _;

use super::{
    InspectionDeadline, digest_arguments, digest_open_file_before, process_group_is_absent,
};

#[test]
fn canonical_arguments_keep_space_boundaries() {
    assert_ne!(
        digest_arguments(&["a b".to_owned(), "c".to_owned()]),
        digest_arguments(&["a".to_owned(), "b c".to_owned()])
    );
}

#[test]
fn canonical_arguments_keep_empty_arguments() {
    assert_ne!(
        digest_arguments(&["a".to_owned(), String::new(), "b".to_owned()]),
        digest_arguments(&["a".to_owned(), "b".to_owned()])
    );
}

#[test]
fn process_group_absence_requires_kernel_esrch() {
    let mut child = std::process::Command::new("/bin/sleep");
    child.arg("60").process_group(0);
    let mut child = child.spawn().expect("spawn isolated group member");
    let group = child.id();
    let present_probe = process_group_is_absent(group);
    let stop = child.kill();
    let reap = child.wait();

    stop.expect("stop isolated group member");
    reap.expect("reap isolated group member");
    assert!(!present_probe.expect("probe live process group"));
    assert!(process_group_is_absent(group).expect("probe absent process group"));
}

#[test]
fn executable_hash_refuses_an_expired_inspection_deadline() {
    let mut file = tempfile::tempfile().expect("create executable fixture");
    let error = digest_open_file_before(
        &mut file,
        std::path::Path::new("executable-fixture"),
        InspectionDeadline::new(std::time::Instant::now(), "inspect test executable"),
    )
    .expect_err("reject expired executable inspection");

    assert!(matches!(
        error,
        crate::AviateSupervisorError::Timeout {
            operation: "inspect test executable"
        }
    ));
}
