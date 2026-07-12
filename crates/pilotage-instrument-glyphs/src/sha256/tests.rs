//! SHA-256 correctness against the published FIPS 180-4 example vectors.

#![allow(clippy::expect_used, clippy::panic)]

use super::sha256;
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
        hex(&sha256(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn abc() {
    assert_eq!(
        hex(&sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn two_block_message() {
    let msg = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
    assert_eq!(
        hex(&sha256(msg)),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

#[test]
fn one_million_a() {
    let msg: Vec<u8> = std::iter::repeat_n(b'a', 1_000_000).collect();
    assert_eq!(
        hex(&sha256(&msg)),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn const_evaluation_matches_runtime() {
    const DIGEST: [u8; 32] = sha256(b"pilotage");
    assert_eq!(DIGEST, sha256(b"pilotage"));
}
