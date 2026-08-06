//! ADR-0026 posture acceptance (#256): two sources with genuinely
//! different group sets — a data-gateway bridge and a flight controller,
//! neither a subset of the other — drive the same state model with zero
//! dead fields, and every unfed group resolves `Missing` with no
//! producer opt-in. This is what proves the contract is open rather
//! than merely open-to-FCs.

#![allow(clippy::expect_used, clippy::panic)]

use super::{CAPACITY, decode_state, encode_state, fixtures};
use crate::group_id::GroupId;
use crate::signal::{FreshnessPolicy, SignalStatus};
use crate::{AircraftState, resolve};
use std::vec::Vec;

fn tags_of(state: &AircraftState) -> Vec<u8> {
    let mut buf = [0u8; CAPACITY];
    let len = encode_state(state, &mut buf).expect("fixture fits");
    let frame = &buf[..len];
    let mut tags = Vec::new();
    let mut offset = 2usize;
    for _ in 0..frame[1] {
        tags.push(frame[offset]);
        let payload = u16::from_le_bytes([frame[offset + 1], frame[offset + 2]]) as usize;
        offset += 3 + payload;
    }
    tags
}

#[test]
fn the_two_postures_carry_disjoint_extras_and_no_dead_fields() {
    // The gateway supplies guidance the FC lacks; the FC supplies air,
    // heading, and dynamics the gateway lacks. Each frame carries
    // exactly its own groups — absence is an absent tag, not a zeroed
    // slot no other source uses.
    let gateway = tags_of(&fixtures::data_gateway());
    let fc = tags_of(&fixtures::flight_controller());
    assert_eq!(
        gateway,
        std::vec![0x02, 0x04, 0x07, 0x08],
        "kinematics, nav, trust, altitude"
    );
    assert_eq!(
        fc,
        std::vec![0x01, 0x02, 0x03, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B],
        "everything but nav and monitor text"
    );
    assert!(gateway.contains(&0x04) && !fc.contains(&0x04));
    assert!(fc.contains(&0x03) && !gateway.contains(&0x03));
}

#[test]
fn gateway_groups_resolve_valid_and_unfed_groups_resolve_missing() {
    let frame = {
        let mut buf = [0u8; CAPACITY];
        let len = encode_state(&fixtures::data_gateway(), &mut buf).expect("fits");
        buf[..len].to_vec()
    };
    let state = decode_state(&frame).expect("decodes").state;
    let data = resolve(&state, &FreshnessPolicy::default());

    assert_eq!(data.groups.status(GroupId::Kinematics), SignalStatus::Valid);
    assert_eq!(data.groups.status(GroupId::Nav), SignalStatus::Valid);
    // Unfed groups are Missing by construction — the producer never
    // declared, flagged, or zeroed anything to make this happen.
    for unfed in [
        GroupId::Attitude,
        GroupId::Air,
        GroupId::Wind,
        GroupId::Heading,
        GroupId::Variation,
        GroupId::Dynamics,
        GroupId::MonitorText,
    ] {
        assert_eq!(
            data.groups.status(unfed),
            SignalStatus::Missing,
            "{unfed:?} must be Missing for the gateway"
        );
    }
    // The rendered signals agree: no airspeed, no heading rose, but a
    // live CDI with its waypoint idents.
    assert_eq!(data.ias_kt.status, SignalStatus::Missing);
    assert_eq!(data.heading.value_rad.status, SignalStatus::Missing);
    assert_eq!(data.nav.status, SignalStatus::Valid);
    assert_eq!(data.nav.data.to_ident.as_str(), "WPT-3");
}

#[test]
fn flight_controller_groups_resolve_valid_and_nav_stays_missing() {
    let frame = {
        let mut buf = [0u8; CAPACITY];
        let len = encode_state(&fixtures::flight_controller(), &mut buf).expect("fits");
        buf[..len].to_vec()
    };
    let state = decode_state(&frame).expect("decodes").state;
    let data = resolve(&state, &FreshnessPolicy::default());

    for fed in [
        GroupId::Attitude,
        GroupId::Kinematics,
        GroupId::Air,
        GroupId::Wind,
        GroupId::Heading,
        GroupId::Variation,
        GroupId::Dynamics,
    ] {
        assert_eq!(
            data.groups.status(fed),
            SignalStatus::Valid,
            "{fed:?} must be Valid for the flight controller"
        );
    }
    assert_eq!(data.groups.status(GroupId::Nav), SignalStatus::Missing);
    assert_eq!(data.nav.status, SignalStatus::Missing);
    assert_eq!(data.roll_rad.status, SignalStatus::Valid);
    assert_eq!(data.ias_kt.status, SignalStatus::Valid);
}
