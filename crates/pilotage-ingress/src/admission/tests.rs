#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use crate::source::{MeasurementClock, SourceIncarnation, SourceIntegrity, SourceRole};

fn incarnation(tag: u8) -> SourceIncarnation {
    let mut bytes = [0u8; 16];
    bytes[15] = tag;
    SourceIncarnation::new(bytes)
}

fn stamp(epoch: u32, sequence: u32, acquired_at_ns: u64) -> MeasurementStamp {
    MeasurementStamp {
        role: SourceRole::OperationalEstimate,
        integrity: SourceIntegrity::Authenticated,
        source_id: 7,
        source_incarnation: incarnation(1),
        source_epoch: epoch,
        sequence,
        acquired_at_ns,
        clock: MeasurementClock::VehicleBoot,
    }
}

fn allow(_: &MeasurementStamp) -> bool {
    true
}

fn deny(_: &MeasurementStamp) -> bool {
    false
}

/// Admits a stamp the test depends on having been accepted.
///
/// A setup line that silently stopped establishing state would leave the
/// assertion below it passing for the wrong reason.
fn establish(gate: &mut SourceGate, stamp: &MeasurementStamp) {
    assert!(
        matches!(gate.admit(stamp, allow), Admission::Accepted { .. }),
        "setup stamp must be admitted"
    );
}

#[test]
fn the_first_group_from_an_authorized_attachment_is_admitted() {
    let mut gate = SourceGate::new();
    assert_eq!(
        Admission::Accepted {
            identity_changed: true
        },
        gate.admit(&stamp(1, 10, 1_000), allow)
    );
    assert_eq!(1, gate.accepted());
}

#[test]
fn an_unauthorized_attachment_is_refused_and_leaves_no_state() {
    let mut gate = SourceGate::new();
    assert_eq!(
        Admission::Rejected(RejectReason::UnauthorizedIncarnation),
        gate.admit(&stamp(1, 10, 1_000), deny)
    );
    assert!(gate.last_admitted().is_none());
    assert_eq!(1, gate.rejected());
}

#[test]
fn advancing_sequence_within_one_epoch_is_admitted_without_clearing_state() {
    let mut gate = SourceGate::new();
    establish(&mut gate, &stamp(1, 10, 1_000));
    assert_eq!(
        Admission::Accepted {
            identity_changed: false
        },
        gate.admit(&stamp(1, 11, 1_100), allow)
    );
}

#[test]
fn a_duplicate_sequence_cannot_refresh_state() {
    let mut gate = SourceGate::new();
    establish(&mut gate, &stamp(1, 10, 1_000));
    // A re-publication carries the complete original stamp, so it is a
    // duplicate rather than a new measurement, however late it arrives.
    assert_eq!(
        Admission::Rejected(RejectReason::DuplicateSequence),
        gate.admit(&stamp(1, 10, 9_999), allow)
    );
    assert_eq!(10, gate.last_admitted().expect("a stamp").sequence);
}

#[test]
fn a_reordered_sequence_is_refused() {
    let mut gate = SourceGate::new();
    establish(&mut gate, &stamp(1, 10, 1_000));
    assert_eq!(
        Admission::Rejected(RejectReason::ReorderedSequence),
        gate.admit(&stamp(1, 9, 1_100), allow)
    );
}

#[test]
fn sequence_comparison_survives_the_wrap_boundary() {
    let mut gate = SourceGate::new();
    establish(&mut gate, &stamp(1, u32::MAX, 1_000));
    // Wrapping past the maximum is the next measurement, not an ancient one.
    assert_eq!(
        Admission::Accepted {
            identity_changed: false
        },
        gate.admit(&stamp(1, 0, 1_100), allow)
    );
    // ...and the value before the wrap is still older.
    assert_eq!(
        Admission::Rejected(RejectReason::ReorderedSequence),
        gate.admit(&stamp(1, u32::MAX - 1, 1_200), allow)
    );
}

#[test]
fn a_newer_epoch_clears_the_previous_identity() {
    let mut gate = SourceGate::new();
    establish(&mut gate, &stamp(1, 500, 5_000));
    // A source reset restarts the sequence; the epoch is what orders it, and
    // the caller must clear the other groups from the previous epoch.
    assert_eq!(
        Admission::Accepted {
            identity_changed: true
        },
        gate.admit(&stamp(2, 0, 10), allow)
    );
}

#[test]
fn an_older_epoch_is_refused() {
    let mut gate = SourceGate::new();
    establish(&mut gate, &stamp(5, 10, 1_000));
    assert_eq!(
        Admission::Rejected(RejectReason::OlderEpoch),
        gate.admit(&stamp(4, 11, 1_100), allow)
    );
}

#[test]
fn epoch_comparison_survives_the_wrap_boundary() {
    let mut gate = SourceGate::new();
    establish(&mut gate, &stamp(u32::MAX, 10, 1_000));
    assert_eq!(
        Admission::Accepted {
            identity_changed: true
        },
        gate.admit(&stamp(0, 0, 1_100), allow)
    );
}

#[test]
fn acquisition_time_may_not_regress_within_one_epoch() {
    let mut gate = SourceGate::new();
    establish(&mut gate, &stamp(1, 10, 5_000));
    assert_eq!(
        Admission::Rejected(RejectReason::AcquisitionRegressed),
        gate.admit(&stamp(1, 11, 4_999), allow)
    );
}

#[test]
fn a_clock_domain_change_within_one_attachment_is_refused() {
    let mut gate = SourceGate::new();
    establish(&mut gate, &stamp(1, 10, 1_000));
    let mut moved = stamp(1, 11, 1_100);
    moved.clock = MeasurementClock::HostMonotonic;
    // Two domains cannot be ordered without an explicit correlation, so the
    // gate refuses rather than inventing one.
    assert_eq!(
        Admission::Rejected(RejectReason::ClockDomainChanged),
        gate.admit(&moved, allow)
    );
}

#[test]
fn a_new_attachment_is_re_authorized_and_clears_the_previous_identity() {
    let mut gate = SourceGate::new();
    establish(&mut gate, &stamp(9, 900, 9_000));
    let mut reattached = stamp(1, 0, 10);
    reattached.source_incarnation = incarnation(2);
    // Epoch ordering is meaningless across incarnations: a lower epoch under a
    // newly authorized attachment is legitimate.
    assert_eq!(
        Admission::Accepted {
            identity_changed: true
        },
        gate.admit(&reattached, allow)
    );
}

#[test]
fn an_unauthorized_reattachment_cannot_displace_the_current_source() {
    let mut gate = SourceGate::new();
    establish(&mut gate, &stamp(1, 10, 1_000));
    let mut reattached = stamp(1, 11, 1_100);
    reattached.source_incarnation = incarnation(2);
    assert_eq!(
        Admission::Rejected(RejectReason::UnauthorizedIncarnation),
        gate.admit(&reattached, deny)
    );
    assert_eq!(
        incarnation(1),
        gate.last_admitted().expect("a stamp").source_incarnation
    );
}

#[test]
fn a_different_role_on_the_same_id_is_a_different_attachment() {
    let mut gate = SourceGate::new();
    establish(&mut gate, &stamp(1, 10, 1_000));
    let mut other = stamp(1, 11, 1_100);
    other.role = SourceRole::SimulationTruth;
    // Ids may collide across roles, so the role must disambiguate rather than
    // letting simulation truth advance an operational stream.
    assert_eq!(
        Admission::Rejected(RejectReason::UnauthorizedIncarnation),
        gate.admit(&other, deny)
    );
}
