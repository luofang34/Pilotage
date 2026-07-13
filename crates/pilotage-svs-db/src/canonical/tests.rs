//! Tests for the canonical byte layout and the recorded content hash.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::{manifest_canonical_bytes, manifest_content_hash, tile_canonical_bytes};
use crate::fixtures;

fn to_hex(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Pins the canonical encoding: the SHA-256 of the fixture manifest's canonical
/// bytes is a recorded constant. Any change to the byte layout (field order,
/// widths, endianness, or which fields are covered) changes this hash and fails
/// here — the encoding cannot drift silently.
#[test]
fn manifest_content_hash_is_stable() {
    let candidate = fixtures::candidate();
    let got = to_hex(&manifest_content_hash(&candidate.manifest));
    let recorded = "253f25080ddce18c905526d61a4e4624e222102dd1a1425f60afeb98ee981815";
    assert_eq!(got, recorded, "canonical manifest encoding changed");
}

#[test]
fn canonical_bytes_are_deterministic() {
    let candidate = fixtures::candidate();
    assert_eq!(
        manifest_canonical_bytes(&candidate.manifest),
        manifest_canonical_bytes(&candidate.manifest)
    );
}

#[test]
fn mutating_a_manifest_field_changes_the_canonical_bytes() {
    let candidate = fixtures::candidate();
    let mut mutated = candidate.manifest.clone();
    mutated.coverage.region.max_lat_deg += 0.5;
    assert_ne!(
        manifest_canonical_bytes(&candidate.manifest),
        manifest_canonical_bytes(&mutated)
    );
}

#[test]
fn signature_bytes_are_excluded_from_the_canonical_form() {
    // Changing only the signature bytes must not change what is signed.
    let candidate = fixtures::candidate();
    let mut other = candidate.manifest.clone();
    other.signature.bytes = [0xEE; 64];
    assert_eq!(
        manifest_canonical_bytes(&candidate.manifest),
        manifest_canonical_bytes(&other)
    );
}

#[test]
fn tile_canonical_bytes_bind_the_key() {
    let tiles = fixtures::tiles();
    let mut relabelled = tiles[0].clone();
    relabelled.key = tiles[1].key;
    // Same payload, different key -> different canonical bytes.
    assert_ne!(
        tile_canonical_bytes(&tiles[0]),
        tile_canonical_bytes(&relabelled)
    );
}
