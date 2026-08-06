#![allow(clippy::expect_used, clippy::panic)]

use pilotage_instrument_state::GroupId;

use super::GroupSet;

#[test]
fn membership_matches_construction() {
    let set = GroupSet::of(&[GroupId::Attitude, GroupId::Nav, GroupId::MonitorText]);
    assert!(set.contains(GroupId::Attitude));
    assert!(set.contains(GroupId::Nav));
    assert!(set.contains(GroupId::MonitorText));
    assert!(!set.contains(GroupId::Wind));
    assert_eq!(set.len(), 3);
    assert!(!set.is_empty());
    assert!(GroupSet::EMPTY.is_empty());
}

#[test]
fn bits_use_wire_tags_as_positions() {
    // The bitset is a wasm/FFI encoding: bit position must equal the
    // group's wire tag, not an enum ordinal.
    let set = GroupSet::of(&[GroupId::Attitude, GroupId::Trust]);
    assert_eq!(set.bits(), (1 << 0x01) | (1 << 0x07));
}
