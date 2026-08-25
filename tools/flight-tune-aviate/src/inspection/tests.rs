use std::os::unix::process::CommandExt as _;

use super::{digest_arguments, process_group_is_absent};

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
