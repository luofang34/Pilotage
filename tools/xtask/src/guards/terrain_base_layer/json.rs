//! The value rules of the checks: equality, order, and the alternative
//! operator, as jq gives them to a style or a plan.
//!
//! jq compares numbers by value, so `1` equals `1.0`, and it puts `null`
//! below every number. A layer that a style does not hold has no index,
//! and that missing index is lower than every present index.

use serde_json::Value;

/// The member `key` of an object; `null` for anything else.
pub(super) fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

/// The value at a path of object keys; `null` where a step is missing.
pub(super) fn at<'a>(value: &'a Value, path: &[&str]) -> &'a Value {
    path.iter().fold(value, |current, key| field(current, key))
}

/// The number a value holds, if it is a number.
pub(super) fn number(value: &Value) -> Option<f64> {
    value.as_f64()
}

/// True when `value` is the number `expected`.
pub(super) fn is_number(value: &Value, expected: f64) -> bool {
    number(value).is_some_and(|actual| actual == expected)
}

/// True when `value` is the string `expected`.
pub(super) fn is_text(value: &Value, expected: &str) -> bool {
    value.as_str() == Some(expected)
}

/// jq equality: numbers by value, arrays by element, objects by key.
pub(super) fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(key, p)| y.get(key).is_some_and(|q| same(p, q)))
        }
        _ => a == b,
    }
}

/// jq's `left // right`: `left` unless it is `null` or `false`.
pub(super) fn or_else<'a>(left: &'a Value, right: &'a Value) -> &'a Value {
    match left {
        Value::Null | Value::Bool(false) => right,
        _ => left,
    }
}

/// The elements of an array; `None` for a value that jq cannot iterate.
pub(super) fn items(value: &Value) -> Option<&[Value]> {
    value.as_array().map(Vec::as_slice)
}

/// jq's `length` of a string, an array, or an object; `null` is 0.
pub(super) fn length(value: &Value) -> Option<usize> {
    match value {
        Value::Null => Some(0),
        Value::String(text) => Some(text.chars().count()),
        Value::Array(list) => Some(list.len()),
        Value::Object(map) => Some(map.len()),
        Value::Bool(_) | Value::Number(_) => None,
    }
}

/// The text `jq -r` writes for a value: a string as it is, and any
/// other value as compact JSON.
pub(super) fn raw_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The layers of a style, in draw order.
pub(super) fn layers(style: &Value) -> &[Value] {
    items(field(style, "layers")).unwrap_or(&[])
}

/// The position of the layer `id` in draw order.
pub(super) fn layer_index(style: &Value, id: &str) -> Option<usize> {
    layers(style)
        .iter()
        .position(|layer| is_text(field(layer, "id"), id))
}

/// The layers with the id `id`.
pub(super) fn layers_with_id<'a>(style: &'a Value, id: &'a str) -> impl Iterator<Item = &'a Value> {
    layers(style)
        .iter()
        .filter(move |layer| is_text(field(layer, "id"), id))
}

/// The number of layers that satisfy `test`.
pub(super) fn count_layers(style: &Value, test: impl Fn(&Value) -> bool) -> usize {
    layers(style).iter().filter(|layer| test(layer)).count()
}

/// jq's order of a value against the number `n`: `null` and booleans sort
/// below every number, strings, arrays, and objects above.
fn order_against(value: &Value, n: f64) -> Option<std::cmp::Ordering> {
    match value {
        Value::Null | Value::Bool(_) => Some(std::cmp::Ordering::Less),
        Value::Number(_) => number(value).and_then(|v| v.partial_cmp(&n)),
        Value::String(_) | Value::Array(_) | Value::Object(_) => Some(std::cmp::Ordering::Greater),
    }
}

/// jq's `value <= n`.
pub(super) fn at_most(value: &Value, n: f64) -> bool {
    order_against(value, n).is_some_and(|order| order.is_le())
}

/// jq's `value >= n`.
pub(super) fn at_least(value: &Value, n: f64) -> bool {
    order_against(value, n).is_some_and(|order| order.is_ge())
}

/// jq's `value > n`.
pub(super) fn above(value: &Value, n: f64) -> bool {
    order_against(value, n).is_some_and(|order| order.is_gt())
}

/// jq's `[values] | max` when the answer is a number: `null` and
/// booleans sort below numbers and drop out. `None` when no value is a
/// number, or when a string, an array, or an object sorts above them.
pub(super) fn max_number<'a>(values: impl Iterator<Item = &'a Value>) -> Option<f64> {
    let mut best: Option<f64> = None;
    for value in values {
        match value {
            Value::Null | Value::Bool(_) => {}
            Value::Number(_) => {
                let v = number(value)?;
                best = Some(best.map_or(v, |b| b.max(v)));
            }
            Value::String(_) | Value::Array(_) | Value::Object(_) => return None,
        }
    }
    best
}

/// The SHA-256 of a file as lowercase hex, or the empty string when the
/// file cannot be read.
pub(super) fn file_digest(path: &std::path::Path) -> String {
    use sha2::{Digest as _, Sha256};
    std::fs::read(path)
        .map(|bytes| {
            Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        })
        .unwrap_or_default()
}
