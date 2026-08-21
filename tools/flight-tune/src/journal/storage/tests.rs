#![allow(clippy::expect_used)]

use super::{MAX_DOCUMENT_BYTES, encode};
use crate::TuneError;

#[test]
fn an_oversized_document_is_rejected_before_a_write() {
    let content = "x".repeat(MAX_DOCUMENT_BYTES as usize);
    let error = encode("oversized test document", &content).expect_err("reject oversized data");

    assert!(matches!(error, TuneError::DocumentTooLarge { .. }));
}
