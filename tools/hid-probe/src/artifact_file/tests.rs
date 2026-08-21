#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::write_new;

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

struct TemporaryArtifact {
    path: PathBuf,
}

impl TemporaryArtifact {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        Self {
            path: std::env::temp_dir().join(format!(
                "pilotage-hid-artifact-{}-{nonce}.json",
                std::process::id()
            )),
        }
    }
}

impl Drop for TemporaryArtifact {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}
