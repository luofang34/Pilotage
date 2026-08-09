//! Bridge proofs over the rlib: enumeration matches the runtime, the
//! render round trip carries the typed outcome, and the digest identity
//! equals the pinned compatibility-tuple values.

#![allow(clippy::expect_used, clippy::panic)]

use indicate_instrument_state::abi::v7::encode_state;
use indicate_instrument_state::{AircraftState, Attitude, Quat, Stamped};
use pilotage_instrument_runtime::RenderStatus;

use crate::{
    InstrumentBridge, composition_digest_hex, composition_slot, composition_slot_count,
    corpus_digest_hex, corpus_version, panel_count, panel_descriptor, scene_digest_hex,
    scene_format_version, state_abi_version,
};

const PINNED_SCENE_DIGEST: &str =
    "f82d905643b48822de25665761ad3e29daa334d937f18b1e98a3e215353cb704";
const PINNED_COMPOSITION_DIGEST: &str =
    "6761e8e1ed137e682530274c8f02353d2ab40e7142a36cd4321a6835323b463c";
const PINNED_CORPUS_DIGEST: &str =
    "1fb8e6de2734ff7506843b05869f39d501f0926599636c6110a7e3b0c6e1625e";

fn attitude_state() -> AircraftState {
    AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat: Quat::IDENTITY,
                rates_rps: [0.0; 3],
            }),
            age_ms: Some(10.0),
        },
        valid: indicate_instrument_state::ValidFlags {
            attitude: true,
            rates: true,
            position: true,
            velocity_horizontal: true,
            velocity_vertical: true,
            ..Default::default()
        },
        ..AircraftState::default()
    }
}

fn encode(state: &AircraftState) -> Vec<u8> {
    let mut block = vec![0u8; indicate_instrument_state::abi::v7::CAPACITY];
    let len = encode_state(state, &mut block).expect("encodes");
    block.truncate(len);
    block
}

#[test]
fn enumeration_matches_the_runtime() {
    assert_eq!(panel_count(), pilotage_instrument_runtime::panel_count());
    let pfd = panel_descriptor(0).expect("panel 0");
    assert_eq!(pfd.id, "pfd");
    assert_eq!(pfd.design_width, 480.0);
    assert_eq!(pfd.design_height, 360.0);
    let hsi = panel_descriptor(1).expect("panel 1");
    assert_eq!(hsi.id, "hsi");
    assert!(panel_descriptor(99).is_none(), "unknown index fails closed");

    assert_eq!(
        composition_slot_count(),
        pilotage_instrument_runtime::composition_slot_count()
    );
    assert_eq!(composition_slot_count(), 2);
    let first = composition_slot(0).expect("slot 0");
    assert_eq!(first.panel, "pfd");
    assert_eq!(
        (first.x, first.y, first.width, first.height),
        (0.0, 0.0, 480.0, 360.0)
    );
    let second = composition_slot(1).expect("slot 1");
    assert_eq!(second.panel, "hsi");
    assert_eq!((second.x, second.y), (480.0, 0.0));
    assert!(composition_slot(9).is_none(), "unknown slot fails closed");
}

#[test]
fn digest_identity_equals_the_pinned_tuple_values() {
    assert_eq!(
        state_abi_version(),
        pilotage_instrument_runtime::abi_version()
    );
    assert_eq!(
        scene_format_version(),
        pilotage_instrument_runtime::scene_format_version()
    );
    assert_eq!(corpus_version(), 4);
    assert_eq!(corpus_digest_hex(), PINNED_CORPUS_DIGEST);
    assert_eq!(scene_digest_hex(), PINNED_SCENE_DIGEST);
    assert_eq!(composition_digest_hex(), PINNED_COMPOSITION_DIGEST);
}

#[test]
fn render_round_trip_carries_the_typed_outcome() {
    let bridge = InstrumentBridge::new();
    assert_eq!(bridge.write_state(&encode(&attitude_state())).status, 0);

    let first = bridge.render(0);
    assert_eq!(first.status, RenderStatus::Ok as u32);
    assert!(!first.scene.is_empty(), "the outcome carries scene bytes");
    assert_eq!(first.generation, 1);
    assert_eq!((first.frame_width, first.frame_height), (480.0, 360.0));

    let second = bridge.render(0);
    assert_eq!(second.status, RenderStatus::Ok as u32);
    assert_eq!(second.generation, 2, "a second success advances generation");
}

#[test]
fn a_truncated_state_fails_with_unchanged_generation() {
    // The state buffer is fixed-capacity and the v7 frame is
    // self-delimiting, so a short write is not a truncation. An
    // over-declared group length is: tag 0x05 claims 65535 payload
    // bytes against the 1024-byte buffer.
    let bridge = InstrumentBridge::new();
    assert_eq!(bridge.write_state(&[7, 1, 0x05, 0xff, 0xff]).status, 0);

    let outcome = bridge.render(0);
    assert_eq!(outcome.status, RenderStatus::StateTruncated as u32);
    assert!(outcome.scene.is_empty(), "a failure carries no scene bytes");
    assert_eq!(outcome.generation, 0, "a failure never advances generation");
}

#[test]
fn state_write_accepts_exact_capacity_and_refuses_capacity_plus_one() {
    let bridge = InstrumentBridge::new();
    let capacity = indicate_instrument_state::abi::v7::CAPACITY;
    assert_eq!(bridge.write_state(&vec![0; capacity]).status, 0);

    let error = bridge.write_state(&vec![0; capacity + 1]);
    assert_eq!(error.status, 1);
    assert_eq!(error.actual, (capacity + 1) as u64);
    assert_eq!(error.capacity, capacity as u64);
}

#[test]
fn oversized_valid_prefix_is_not_truncated_into_an_accepted_frame() {
    let bridge = InstrumentBridge::new();
    let encoded = encode(&attitude_state());
    assert_eq!(bridge.write_state(&encoded).status, 0);
    let first = bridge.render(0);
    assert_eq!(first.status, RenderStatus::Ok as u32);

    let mut oversized = encoded;
    oversized.resize(indicate_instrument_state::abi::v7::CAPACITY + 1, 0);
    assert_eq!(bridge.write_state(&oversized).status, 1);
    let refused = bridge.render(0);
    assert_ne!(refused.status, RenderStatus::Ok as u32);
    assert!(refused.scene.is_empty());
    assert_eq!(refused.generation, first.generation);
}
