//! Ground truth for the Annex-B classification: synthetic NAL layouts pin the
//! iterator and every classification branch; the recorded encoder fixture
//! (tests/fixtures, produced by libx264 — see its provenance note) pins the
//! same behavior over real bytes shared with the wasm conformance tests.
#![allow(clippy::expect_used, clippy::panic)]

use sha2::{Digest, Sha256};

use super::{ChunkClass, KeyframeFault, classify_chunk, nal_units};

/// The recorded Annex-B baseline fixture (real libx264 output).
const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/h264-annexb-baseline.h264");

/// The fixture's pinned content digest, from its provenance note.
const FIXTURE_SHA256: &str = "84d843b4334d9a5a2aec482d0a56f4fb60ce450a5c87b6f8414eb9d3a39fe6c7";

/// One NAL unit with a 4-byte start code, the given type, and body bytes.
fn nal(nal_type: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0, 0, 0, 1, nal_type];
    out.extend_from_slice(body);
    out
}

/// A minimal SPS whose profile/constraint/level bytes are the given triple.
fn sps(profile: u8, constraint: u8, level: u8) -> Vec<u8> {
    nal(7, &[profile, constraint, level, 0xff])
}

fn access_unit(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.concat()
}

#[test]
fn nal_units_walk_both_start_code_forms_in_order() {
    let mut au = access_unit(&[sps(0x42, 0xe0, 0x1e), nal(8, &[0x01]), nal(5, &[0x02])]);
    // Append one 3-byte-start-code unit so both forms are covered.
    au.extend_from_slice(&[0, 0, 1, 1, 0xaa]);
    let types: Vec<u8> = nal_units(&au).map(|n| n.nal_type).collect();
    assert_eq!(types, [7, 8, 5, 1]);
}

#[test]
fn garbage_and_truncated_start_codes_yield_no_units() {
    assert_eq!(nal_units(&[9, 9, 9, 9, 9]).count(), 0);
    // A start code at the very end has no header byte to classify.
    assert_eq!(nal_units(&[0, 0, 0, 1]).count(), 0);
    assert_eq!(nal_units(&[0, 0, 1]).count(), 0);
    assert_eq!(nal_units(&[]).count(), 0);
}

#[test]
fn a_non_idr_unit_is_a_delta_frame() {
    let au = access_unit(&[nal(1, &[0x33])]);
    assert_eq!(classify_chunk(&au), ChunkClass::Delta);
    // Malformed bytes classify as delta too: nothing a session can start on.
    assert_eq!(classify_chunk(&[9, 9, 9]), ChunkClass::Delta);
}

#[test]
fn a_keyframe_with_both_parameter_sets_names_its_codec() {
    let au = access_unit(&[sps(0x42, 0xe0, 0x1e), nal(8, &[0x01]), nal(5, &[0x02])]);
    assert_eq!(
        classify_chunk(&au),
        ChunkClass::Keyframe {
            codec: "avc1.42e01e".to_string()
        }
    );
}

#[test]
fn a_keyframe_missing_either_parameter_set_is_undecodable() {
    let no_sps = access_unit(&[nal(8, &[0x01]), nal(5, &[0x02])]);
    assert_eq!(
        classify_chunk(&no_sps),
        ChunkClass::UndecodableKeyframe {
            fault: KeyframeFault::MissingSps
        }
    );
    let no_pps = access_unit(&[sps(0x42, 0xe0, 0x1e), nal(5, &[0x02])]);
    assert_eq!(
        classify_chunk(&no_pps),
        ChunkClass::UndecodableKeyframe {
            fault: KeyframeFault::MissingPps
        }
    );
}

#[test]
fn an_sps_truncated_before_its_profile_bytes_is_undecodable() {
    // SPS header byte present but the buffer ends inside the profile triple.
    let au = access_unit(&[nal(8, &[0x01]), nal(5, &[0x02]), vec![0, 0, 0, 1, 7, 0x42]]);
    assert_eq!(
        classify_chunk(&au),
        ChunkClass::UndecodableKeyframe {
            fault: KeyframeFault::SpsTooShort
        }
    );
}

#[test]
fn the_first_sps_in_stream_order_names_the_codec() {
    let au = access_unit(&[
        sps(0x42, 0xc0, 0x0a),
        sps(0x64, 0x00, 0x28),
        nal(8, &[0x01]),
        nal(5, &[0x02]),
    ]);
    assert_eq!(
        classify_chunk(&au),
        ChunkClass::Keyframe {
            codec: "avc1.42c00a".to_string()
        }
    );
}

#[test]
fn every_fault_reason_is_stable_and_distinct() {
    let reasons = [
        KeyframeFault::MissingSps.reason(),
        KeyframeFault::MissingPps.reason(),
        KeyframeFault::SpsTooShort.reason(),
    ];
    assert!(reasons.iter().all(|r| !r.is_empty()));
    assert_ne!(reasons[0], reasons[1]);
    assert_ne!(reasons[1], reasons[2]);
}

#[test]
fn the_recorded_fixture_matches_its_provenance_and_classifies_decodable() {
    let digest = format!("{:x}", Sha256::digest(FIXTURE));
    assert_eq!(
        digest, FIXTURE_SHA256,
        "fixture bytes drifted from provenance"
    );
    let types: Vec<u8> = nal_units(FIXTURE).map(|n| n.nal_type).collect();
    assert_eq!(
        types,
        [7, 8, 6, 5, 1, 1, 1, 1],
        "SPS, PPS, SEI, IDR, then delta slices"
    );
    assert_eq!(
        classify_chunk(FIXTURE),
        ChunkClass::Keyframe {
            codec: "avc1.42c00a".to_string()
        }
    );
    // The delta tail (everything after the IDR access unit's slices) still
    // classifies as delta when presented alone.
    let idr_at = nal_units(FIXTURE)
        .find(|n| n.nal_type == 5)
        .expect("fixture has an IDR")
        .header_offset;
    let first_delta = nal_units(FIXTURE)
        .find(|n| n.nal_type == 1 && n.header_offset > idr_at)
        .expect("fixture has delta slices");
    let tail = &FIXTURE[first_delta.header_offset - 4..];
    assert_eq!(classify_chunk(tail), ChunkClass::Delta);
}
