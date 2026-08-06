//! The overflow contract: a label past its capacity formats to the
//! empty string — wrong-but-safe beats a panic in drawing code.

#![allow(clippy::expect_used, clippy::panic)]

#[test]
fn labels_within_capacity_format_exactly() {
    let label = fmt_label!(8, "{:03}\u{b0}", 42);
    assert_eq!(label.as_str(), "042\u{b0}");
}

#[test]
fn overflow_yields_the_empty_string_not_a_panic() {
    let overflowing = fmt_label!(4, "{}", 123456789);
    assert_eq!(overflowing.as_str(), "");
    // Partial writes before the overflow are also discarded from view:
    // the readout shows nothing rather than a truncated number that
    // reads as a different value.
    let mixed = fmt_label!(6, "ALT {}", 10500);
    assert_eq!(mixed.as_str(), "");
}
