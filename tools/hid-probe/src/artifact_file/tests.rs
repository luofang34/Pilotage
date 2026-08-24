#![allow(clippy::expect_used)]

use std::path::PathBuf;

use super::{
    read_bounded, validate_artifact_byte_count, write_new, write_new_bounded, write_new_json,
};

#[test]
fn an_existing_artifact_is_not_overwritten() {
    let artifact = TemporaryArtifact::new();
    write_new(&artifact.path, b"approved\n").expect("write first artifact");

    assert!(write_new(&artifact.path, b"replacement\n").is_err());
    assert_eq!(
        std::fs::read(&artifact.path).expect("read artifact"),
        b"approved\n"
    );
}

#[test]
fn a_read_stops_at_the_artifact_limit() {
    let artifact = TemporaryArtifact::new();
    write_new(&artifact.path, b"12345").expect("write artifact");

    assert!(read_bounded(&artifact.path, 4).is_err());
}

#[test]
fn streamed_json_keeps_the_reviewed_pretty_format() {
    let artifact = TemporaryArtifact::new();
    let value = serde_json::json!({"sample": [1, 2]});
    write_new_json(&artifact.path, &value).expect("stream JSON");
    let actual = std::fs::read_to_string(&artifact.path).expect("read streamed JSON");
    let expected = format!(
        "{}\n",
        serde_json::to_string_pretty(&value).expect("expected JSON")
    );
    assert_eq!(actual, expected);
}

#[test]
fn bounded_output_checks_the_complete_bytes_before_create_new() {
    let artifact = TemporaryArtifact::new();
    assert!(validate_artifact_byte_count(&artifact.path, 4, 4).is_ok());
    assert!(write_new_bounded(&artifact.path, b"12345", 4).is_err());
    assert!(!artifact.path.exists());
}

struct TemporaryArtifact {
    path: PathBuf,
    _directory: tempfile::TempDir,
}

impl TemporaryArtifact {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("create unique artifact directory");
        let path = directory.path().join("artifact.json");
        Self {
            path,
            _directory: directory,
        }
    }
}
