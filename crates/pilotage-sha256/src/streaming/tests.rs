//! Streaming-context equivalence with the one-shot digest and the
//! published FIPS 180-4 vectors.

#![allow(clippy::expect_used, clippy::panic)]

use super::Sha256Ctx;
use crate::sha256;
use std::vec::Vec;

fn hex(bytes: &[u8; 32]) -> std::string::String {
    use std::fmt::Write as _;
    let mut s = std::string::String::new();
    for b in bytes {
        write!(s, "{b:02x}").expect("write to String");
    }
    s
}

#[test]
fn empty_input() {
    assert_eq!(
        hex(&Sha256Ctx::new().finalize()),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn abc_single_update() {
    let mut ctx = Sha256Ctx::new();
    ctx.update(b"abc");
    assert_eq!(
        hex(&ctx.finalize()),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn one_million_a_in_uneven_updates() {
    // 977 is coprime with the 64-byte block size, so the carried block
    // cycles through every offset against the published one-million-'a'
    // vector (the under-fill branch is covered by the split-point sweep,
    // whose short tails land in a partly filled block).
    let chunk: Vec<u8> = std::iter::repeat_n(b'a', 977).collect();
    let mut ctx = Sha256Ctx::new();
    let mut fed = 0usize;
    while fed < 1_000_000 {
        let take = chunk.len().min(1_000_000 - fed);
        ctx.update(&chunk[..take]);
        fed += take;
    }
    assert_eq!(
        hex(&ctx.finalize()),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn every_split_point_matches_the_one_shot_digest() {
    let msg: Vec<u8> = (0..=200u8).collect();
    let reference = sha256(&msg);
    for split in 0..=msg.len() {
        let mut ctx = Sha256Ctx::new();
        ctx.update(&msg[..split]);
        ctx.update(&msg[split..]);
        assert_eq!(ctx.finalize(), reference, "split at {split}");
    }
}

#[test]
fn every_length_matches_the_one_shot_digest() {
    // Lengths 0..=130 cross both padding boundaries (55/56 and 63/64)
    // and a full block edge.
    let msg: Vec<u8> = (0..130u32)
        .map(|i| (i.wrapping_mul(37) % 251) as u8)
        .collect();
    for len in 0..=msg.len() {
        let mut ctx = Sha256Ctx::new();
        ctx.update(&msg[..len]);
        assert_eq!(ctx.finalize(), sha256(&msg[..len]), "length {len}");
    }
}

#[test]
fn default_is_a_fresh_context() {
    assert_eq!(Sha256Ctx::default().finalize(), sha256(b""));
}

#[test]
fn a_cloned_context_forks_the_stream_mid_block() {
    let mut base = Sha256Ctx::new();
    base.update(b"shared prefix crossing no block boundary");
    let mut left = base.clone();
    let mut right = base;
    left.update(b" then left");
    right.update(b" then right, long enough to cross a 64-byte block boundary after the fork");
    assert_eq!(
        left.finalize(),
        sha256(b"shared prefix crossing no block boundary then left")
    );
    assert_eq!(
        right.finalize(),
        sha256(
            b"shared prefix crossing no block boundary then right, long enough to cross a 64-byte block boundary after the fork"
        )
    );
}

#[test]
fn digests_agree_with_the_reference_implementation() {
    use sha2::{Digest as _, Sha256};
    // Deterministic pseudo-random content over lengths crossing several
    // block and padding boundaries, checked against the independent
    // `sha2` implementation so correctness does not rest only on the
    // published vectors and self-consistency.
    let mut seed: u32 = 0x1234_5678;
    let mut bytes = Vec::new();
    for length in 0..=300usize {
        while bytes.len() < length {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            bytes.push((seed >> 24) as u8);
        }
        let expected: [u8; 32] = Sha256::digest(&bytes[..length]).into();
        assert_eq!(sha256(&bytes[..length]), expected, "length {length}");
        let mut ctx = Sha256Ctx::new();
        for chunk in bytes[..length].chunks(37) {
            ctx.update(chunk);
        }
        assert_eq!(ctx.finalize(), expected, "streamed length {length}");
    }
}
