#![allow(clippy::expect_used, clippy::panic)]

use super::{QUARANTINE_REASON_DOMAIN, quarantine_reason_digest};

#[test]
fn one_reason_has_one_identity() {
    let reason = "terminal receipt 00 has quarantine class execution_failure";

    assert_eq!(
        quarantine_reason_digest(reason),
        quarantine_reason_digest(reason)
    );
}

#[test]
fn a_changed_reason_changes_its_identity() {
    let execution = quarantine_reason_digest("terminal receipt 00 has quarantine class execution");
    let recovery = quarantine_reason_digest("terminal receipt 00 has quarantine class recovery");

    assert_ne!(execution, recovery);
}

#[test]
fn whitespace_is_part_of_the_reason_identity() {
    assert_ne!(
        quarantine_reason_digest("execution failure"),
        quarantine_reason_digest("execution failure ")
    );
}

#[test]
fn the_domain_separates_a_reason_from_its_bare_bytes() {
    let reason = "execution failure";
    let bare = crate::identity::digest_bytes(reason.as_bytes());

    assert_ne!(quarantine_reason_digest(reason), bare);
    assert!(QUARANTINE_REASON_DOMAIN.ends_with(b"\0"));
}
