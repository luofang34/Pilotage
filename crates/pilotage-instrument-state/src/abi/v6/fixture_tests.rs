//! Golden-frame pinning: the committed hex fixtures in
//! `crates/pilotage-instrument-state/fixtures/` are the canonical v6
//! encodings of the shared
//! posture fixtures, byte for byte. The JS state writer is pinned
//! against the same files, so a drift on either side of the boundary
//! turns this red. On an intentional ABI change, regenerate with
//! `cargo xtask gen-state-fixture`.

#![allow(clippy::expect_used, clippy::panic)]

use super::{CAPACITY, decode_state, encode_state, fixtures};
use crate::aircraft::AircraftState;
use std::string::String;
use std::vec::Vec;

const FULL_HEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/state-abi-v6.full.hex"
));
const GATEWAY_HEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/state-abi-v6.data-gateway.hex"
));
const FC_HEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/state-abi-v6.flight-controller.hex"
));

fn hex_of(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    let mut out = String::new();
    for byte in bytes {
        write!(out, "{byte:02x}").expect("write to String");
    }
    out
}

fn bytes_of(hex: &str) -> Vec<u8> {
    let hex = hex.trim();
    assert!(hex.len().is_multiple_of(2), "odd hex length");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex digit"))
        .collect()
}

fn assert_pinned(state: &AircraftState, committed: &str, name: &str) {
    let mut buf = [0u8; CAPACITY];
    let len = encode_state(state, &mut buf).expect("fixture fits");
    assert_eq!(
        hex_of(&buf[..len]),
        committed.trim(),
        "{name}: encoding drifted from the committed golden frame; if the \
         change is intentional, run `cargo xtask gen-state-fixture`"
    );
    let report = decode_state(&bytes_of(committed)).expect("golden frame decodes");
    assert_eq!(&report.state, state, "{name}: decode disagrees");
    assert_eq!(report.unknown_groups, 0);
    assert_eq!(report.extended_groups, 0);
}

#[test]
fn full_frame_is_pinned() {
    assert_pinned(&fixtures::full(), FULL_HEX, "full");
}

#[test]
fn data_gateway_frame_is_pinned() {
    assert_pinned(&fixtures::data_gateway(), GATEWAY_HEX, "data-gateway");
}

#[test]
fn flight_controller_frame_is_pinned() {
    assert_pinned(&fixtures::flight_controller(), FC_HEX, "flight-controller");
}
