//! The decoder refuses fields it does not know, at every depth.

use super::{assert_unknown_field_rejected, valid_document};

#[test]
fn decoder_rejects_unknown_nested_fields() {
    let document = valid_document();
    assert_unknown_field_rejected(&document, |value| {
        value["phases"][0]["unknown"] = serde_json::Value::Bool(true);
    });
    assert_unknown_field_rejected(&document, |value| {
        value["phases"][0]["action"]["action"]["unknown"] = serde_json::Value::Bool(true);
    });
}
