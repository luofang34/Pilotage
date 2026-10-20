#[test]
fn descriptors_reject_other_profiles_without_hardware() {
    assert_eq!(super::profile_test(), 0);
}

#[test]
fn native_cleanup_waits_for_callbacks_and_failed_reads_publish_no_bytes() {
    assert_eq!(super::lifecycle_test(), 0);
}

#[test]
fn native_write_checks_the_remaining_deadline_before_calling_iokit() {
    assert_eq!(super::deadline_test(), 0);
}
#[test]
fn registry_discovery_rejects_missing_duplicate_and_wrong_interfaces() {
    assert_eq!(super::discovery_test(), 0);
}
