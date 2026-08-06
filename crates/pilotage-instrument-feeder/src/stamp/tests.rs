//! Stamp legality and wrap-safe serial ordering.

#![allow(clippy::expect_used, clippy::panic)]

use super::{
    CLOCK_HOST_MONOTONIC, CLOCK_SIMULATION, CLOCK_VEHICLE_BOOT, ROLE_FC_STATE,
    ROLE_NAVIGATION_SOLUTION, ROLE_OPERATIONAL_ESTIMATE, ROLE_SIMULATION_TRUTH, RawStamp,
    StampFault, serial_is_newer, stamp_fault_for_role,
};

fn stamp(role: u8, clock: u8) -> RawStamp {
    RawStamp {
        role,
        integrity: 2,
        source_id: 9,
        incarnation: [7; 16],
        epoch: 1,
        sequence: 1,
        acquired_at_ns: 1_000,
        clock,
    }
}

#[test]
fn roles_accept_only_their_legal_clocks() {
    let cases = [
        (ROLE_OPERATIONAL_ESTIMATE, CLOCK_VEHICLE_BOOT, true),
        (ROLE_OPERATIONAL_ESTIMATE, CLOCK_SIMULATION, true),
        (ROLE_OPERATIONAL_ESTIMATE, CLOCK_HOST_MONOTONIC, false),
        (ROLE_SIMULATION_TRUTH, CLOCK_SIMULATION, true),
        (ROLE_SIMULATION_TRUTH, CLOCK_VEHICLE_BOOT, false),
        (ROLE_FC_STATE, CLOCK_HOST_MONOTONIC, true),
        (ROLE_FC_STATE, CLOCK_SIMULATION, false),
        (ROLE_NAVIGATION_SOLUTION, CLOCK_HOST_MONOTONIC, true),
        (ROLE_NAVIGATION_SOLUTION, CLOCK_VEHICLE_BOOT, false),
    ];
    for (role, clock, ok) in cases {
        let fault = stamp_fault_for_role(&stamp(role, clock), role);
        assert_eq!(fault.is_none(), ok, "role {role} clock {clock}");
        if !ok {
            assert_eq!(fault, Some(StampFault::IllegalClock));
        }
    }
}

#[test]
fn role_mismatch_and_unknown_integrity_fail_closed() {
    let s = stamp(ROLE_OPERATIONAL_ESTIMATE, CLOCK_SIMULATION);
    assert_eq!(
        stamp_fault_for_role(&s, ROLE_FC_STATE),
        Some(StampFault::RoleMismatch)
    );
    let mut bad = s;
    bad.integrity = 0;
    assert_eq!(
        stamp_fault_for_role(&bad, ROLE_OPERATIONAL_ESTIMATE),
        Some(StampFault::UnknownIntegrity)
    );
}

#[test]
fn serial_ordering_is_wrap_safe() {
    assert!(serial_is_newer(1, 0));
    assert!(!serial_is_newer(0, 0));
    assert!(!serial_is_newer(0, 1));
    // u32::MAX -> 0 is an advance, not a regression.
    assert!(serial_is_newer(0, u32::MAX));
    assert!(!serial_is_newer(u32::MAX, 0));
    // Half-range distances are ambiguous and refused.
    assert!(!serial_is_newer(0x8000_0000, 0));
}
