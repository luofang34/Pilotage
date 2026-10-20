#[test]
fn descriptors_reject_other_profiles_without_hardware() {
    // SAFETY: The native test only reads fixed in-process fixture arrays.
    assert_eq!(unsafe { super::dji_usb_profile_test() }, 0);
}

#[test]
fn native_cleanup_waits_for_callbacks_and_failed_reads_publish_no_bytes() {
    // SAFETY: The native test uses in-process fake vtables and byte arrays.
    assert_eq!(unsafe { super::dji_usb_lifecycle_test() }, 0);
}

#[test]
fn native_write_checks_the_remaining_deadline_before_calling_iokit() {
    // SAFETY: The native test uses an in-process fake write vtable and byte array.
    assert_eq!(unsafe { super::dji_usb_deadline_test() }, 0);
}
#[test]
fn registry_discovery_rejects_missing_duplicate_and_wrong_interfaces() {
    // SAFETY: The native test only evaluates in-process property dictionaries.
    assert_eq!(unsafe { super::dji_usb_discovery_test() }, 0);
}
