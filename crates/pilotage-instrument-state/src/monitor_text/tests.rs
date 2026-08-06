//! Monitor-text validation, wire atoms, and the fail-closed malformed
//! paths.

#![allow(clippy::expect_used, clippy::panic)]

use super::{MonitorText, TextError, TextLine};

#[test]
fn valid_lines_round_trip() {
    for text in [
        "",
        "ENG 1 OK",
        "FUEL 82.5",
        "A-B",
        "0123456789ABCDEF0123456789ABCDEF",
    ] {
        let line = TextLine::new(text).expect("valid line");
        assert_eq!(line.as_str(), text);
        assert_eq!(TextLine::from_wire(&line.to_wire()), line);
    }
}

#[test]
fn over_length_and_charset_violations_are_rejected() {
    let long = "0123456789ABCDEF0123456789ABCDEF0";
    assert_eq!(TextLine::new(long), Err(TextError::TooLong));
    assert_eq!(TextLine::new("eng"), Err(TextError::Charset { byte: b'e' }));
    assert_eq!(TextLine::new("A:B"), Err(TextError::Charset { byte: b':' }));
}

#[test]
fn malformed_wire_atoms_decode_to_invalid() {
    let mut wire = [0u8; 33];
    wire[0] = 34;
    assert!(TextLine::from_wire(&wire).is_invalid());
    let mut wire = [0u8; 33];
    wire[0] = 1;
    wire[1] = b'a';
    assert!(TextLine::from_wire(&wire).is_invalid());
    let mut wire = [0u8; 33];
    wire[0] = 1;
    wire[1] = b'A';
    wire[32] = 7;
    assert!(TextLine::from_wire(&wire).is_invalid());
}

#[test]
fn monitor_text_bounds_its_lines() {
    let line = TextLine::new("OK").expect("valid");
    let lines = [line; 9];
    assert_eq!(MonitorText::new(1, &lines), Err(TextError::TooManyLines));
    let text = MonitorText::new(7, &lines[..2]).expect("two lines fit");
    assert_eq!(text.revision, 7);
    assert_eq!(text.lines().len(), 2);
    assert!(!text.is_malformed());
}

#[test]
fn an_impossible_wire_count_marks_the_channel_malformed() {
    let text = MonitorText::from_wire(3, 9, [TextLine::EMPTY; MonitorText::MAX_LINES]);
    assert!(text.is_malformed());
    assert!(text.lines().is_empty());
}

#[test]
fn an_invalid_line_marks_the_channel_malformed() {
    let mut slots = [TextLine::EMPTY; MonitorText::MAX_LINES];
    slots[0] = TextLine::INVALID;
    let text = MonitorText::from_wire(3, 1, slots);
    assert!(text.is_malformed());
}

#[test]
fn an_invalid_line_in_an_unused_slot_still_fails_the_channel() {
    // The wire decodes every atom; corruption hiding past line_count
    // must not fail-open just because nothing displays it.
    let mut slots = [TextLine::EMPTY; MonitorText::MAX_LINES];
    slots[MonitorText::MAX_LINES - 1] = TextLine::INVALID;
    let text = MonitorText::from_wire(3, 1, slots);
    assert!(text.is_malformed());
}
