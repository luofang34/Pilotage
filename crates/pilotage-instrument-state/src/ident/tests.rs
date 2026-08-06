//! Ident validation, wire round-trips, and the fail-closed INVALID path.

#![allow(clippy::expect_used, clippy::panic)]

use super::{IdentError, IdentStr};

#[test]
fn valid_idents_round_trip() {
    for text in ["", "A", "WPT-1", "KMRY", "12345678", "AB 3"] {
        let ident = IdentStr::new(text).expect("valid ident");
        assert_eq!(ident.as_str(), text);
        assert_eq!(IdentStr::from_wire(&ident.to_wire()), ident);
    }
}

#[test]
fn empty_is_the_default_and_not_invalid() {
    assert_eq!(IdentStr::default(), IdentStr::EMPTY);
    assert!(IdentStr::EMPTY.is_empty());
    assert!(!IdentStr::EMPTY.is_invalid());
}

#[test]
fn over_length_and_charset_violations_are_rejected() {
    assert_eq!(IdentStr::new("123456789"), Err(IdentError::TooLong));
    assert_eq!(
        IdentStr::new("wpt"),
        Err(IdentError::Charset { byte: b'w' })
    );
    assert_eq!(
        IdentStr::new("A.B"),
        Err(IdentError::Charset { byte: b'.' })
    );
}

#[test]
fn malformed_wire_atoms_decode_to_invalid() {
    // Over-length marker.
    let mut wire = [0u8; 9];
    wire[0] = 9;
    assert!(IdentStr::from_wire(&wire).is_invalid());
    // Out-of-charset byte inside the claimed length.
    let mut wire = [0u8; 9];
    wire[0] = 1;
    wire[1] = b'a';
    assert!(IdentStr::from_wire(&wire).is_invalid());
    // Nonzero padding beyond the claimed length.
    let mut wire = [0u8; 9];
    wire[0] = 1;
    wire[1] = b'A';
    wire[8] = 1;
    assert!(IdentStr::from_wire(&wire).is_invalid());
}

#[test]
fn invalid_round_trips_and_reads_empty() {
    let wire = IdentStr::INVALID.to_wire();
    assert_eq!(wire[0], 0xFF);
    let decoded = IdentStr::from_wire(&wire);
    assert!(decoded.is_invalid());
    assert_eq!(decoded.as_str(), "");
}
