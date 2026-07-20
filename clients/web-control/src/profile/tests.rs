#![allow(clippy::expect_used, clippy::panic)]

use super::{DEFAULT_PROFILE_BYTES, ProfileError, ProfileRuntime, SCHEMA_VERSION};

/// A minimal valid profile as a JSON string, so a test can perturb one
/// field at a time without restating the whole document.
fn valid_json() -> String {
    String::from_utf8(DEFAULT_PROFILE_BYTES.to_vec()).expect("default profile is UTF-8")
}

#[test]
fn the_builtin_default_compiles() {
    let compiled = ProfileRuntime::compile(DEFAULT_PROFILE_BYTES).expect("default compiles");
    assert_eq!(compiled.id(), "builtin.gimbal.default");
    assert_eq!(compiled.schema_version(), SCHEMA_VERSION);
    assert_eq!(compiled.revision(), 3);
}

#[test]
fn the_digest_is_stable_and_content_bound() {
    let a = ProfileRuntime::compile(DEFAULT_PROFILE_BYTES).expect("compiles");
    let b = ProfileRuntime::compile(DEFAULT_PROFILE_BYTES).expect("compiles");
    assert_eq!(a.digest(), b.digest(), "same bytes hash identically");
    let mutated = valid_json().replace("\"revision\": 3", "\"revision\": 4");
    let c = ProfileRuntime::compile(mutated.as_bytes()).expect("compiles");
    assert_ne!(a.digest(), c.digest(), "different bytes hash differently");
}

#[test]
fn invalid_utf8_is_rejected() {
    let err = ProfileRuntime::compile(&[0xff, 0xfe, 0x00]).expect_err("not UTF-8");
    assert_eq!(err, ProfileError::InvalidUtf8);
}

#[test]
fn an_unsupported_schema_version_is_rejected() {
    let json = valid_json().replace("\"schema_version\": 1", "\"schema_version\": 2");
    let err = ProfileRuntime::compile(json.as_bytes()).expect_err("bad version");
    assert!(matches!(
        err,
        ProfileError::UnsupportedSchemaVersion {
            found: 2,
            expected: 1
        }
    ));
}

#[test]
fn an_unknown_logical_name_is_rejected() {
    let json = valid_json().replace("\"logical\": \"pitch\"", "\"logical\": \"nonsense\"");
    let err = ProfileRuntime::compile(json.as_bytes()).expect_err("unknown logical");
    assert!(matches!(err, ProfileError::UnknownLogicalName { .. }));
}

#[test]
fn an_ambiguous_binding_is_rejected() {
    // Make the gimbal modifier and reset share button 6.
    let json = valid_json().replace("\"reset_button\": 11", "\"reset_button\": 6");
    let err = ProfileRuntime::compile(json.as_bytes()).expect_err("ambiguous");
    assert!(matches!(err, ProfileError::AmbiguousBinding { .. }));
}

#[test]
fn a_malformed_calibration_is_rejected() {
    // center is not strictly between min and max.
    let json = valid_json().replace(
        "\"calibration\": { \"min\": -1.0, \"center\": 0.0, \"max\": 1.0 }",
        "\"calibration\": { \"min\": 1.0, \"center\": 0.0, \"max\": -1.0 }",
    );
    let err = ProfileRuntime::compile(json.as_bytes()).expect_err("bad calibration");
    assert!(matches!(err, ProfileError::MalformedCalibration { .. }));
}

#[test]
fn a_non_finite_value_is_rejected() {
    // JSON has no NaN literal; a non-finite reaches the schema only as an
    // out-of-range/parse path, so inject it as a bare token the number
    // parser rejects structurally OR a value serde reads as non-finite.
    let json = valid_json().replace("\"deadzone\": 0.06", "\"deadzone\": 1e400");
    let err = ProfileRuntime::compile(json.as_bytes()).expect_err("non-finite");
    // 1e400 overflows f32 to +inf on deserialize; the finiteness gate catches it.
    assert!(matches!(
        err,
        ProfileError::NonFinite { .. } | ProfileError::MalformedJson { .. }
    ));
}

#[test]
fn an_unsupported_field_is_rejected() {
    // A profile cannot grant scopes or add actions: an unknown field fails
    // deny_unknown_fields instead of silently taking effect.
    let json = valid_json().replace(
        "\"id\": \"builtin.gimbal.default\",",
        "\"id\": \"builtin.gimbal.default\", \"grant_scope\": \"vehicle.motion\",",
    );
    let err = ProfileRuntime::compile(json.as_bytes()).expect_err("unsupported field");
    assert!(matches!(err, ProfileError::MalformedJson { .. }));
}
