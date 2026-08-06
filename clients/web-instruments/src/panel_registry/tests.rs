#![allow(clippy::expect_used, clippy::panic)]

use pilotage_instrument_panels::PFD_CONFIG_SCHEMA;
use pilotage_instrument_registry::keys;

use super::splice_v_speeds;

fn entry(key: u16, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&key.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

#[test]
fn splice_preserves_other_keys_and_replaces_the_entry() {
    let mut blob = entry(keys::BACKGROUND_MODE.0, &[1]);
    blob.extend_from_slice(&entry(keys::V_SPEEDS.0, &[9; 20]));
    blob.extend_from_slice(&entry(keys::SVS_QUALITY.0, &[2]));
    let spliced =
        splice_v_speeds(&blob, PFD_CONFIG_SCHEMA, Some([7; 20])).expect("well-formed splice");
    let mut expected = entry(keys::BACKGROUND_MODE.0, &[1]);
    expected.extend_from_slice(&entry(keys::V_SPEEDS.0, &[7; 20]));
    expected.extend_from_slice(&entry(keys::SVS_QUALITY.0, &[2]));
    assert_eq!(spliced, expected, "replaced, not duplicated; others kept");
}

#[test]
fn splice_clears_the_entry_on_none() {
    let mut blob = entry(keys::BACKGROUND_MODE.0, &[1]);
    blob.extend_from_slice(&entry(keys::V_SPEEDS.0, &[9; 20]));
    let spliced = splice_v_speeds(&blob, PFD_CONFIG_SCHEMA, None).expect("well-formed splice");
    assert_eq!(spliced, entry(keys::BACKGROUND_MODE.0, &[1]));
}

#[test]
fn splice_refuses_a_malformed_blob() {
    assert_eq!(splice_v_speeds(&[1, 0, 9], PFD_CONFIG_SCHEMA, None), None);
}
