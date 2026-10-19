//! The guard against a fixture tree: it passes the shipped exemptions,
//! names every response constant outside them, refuses a reused
//! exemption and a public profile entry, and passes the real tree.

#![allow(clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use super::{AVIATE_SRC, check, declaration, reads_as_response};

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("plt_xt_feel_guard_{tag}_{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let fixture = Self { root };
        fixture.write("uplink.rs", "const MAX_DT_S: f32 = 0.1;\n");
        fixture.write(
            "uplink/feel.rs",
            "pub fn shape(value: f32) -> f32 { value }\n",
        );
        fixture.write(
            "adapter/control.rs",
            "const RESET_CLEAR_DEADBAND: f32 = 0.05;\n",
        );
        fixture.write(
            "adapter/pointing.rs",
            "pub(crate) const MAX_YAW_RATE_RPS: f32 = 0.8;\n",
        );
        fixture
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.root.join(AVIATE_SRC).join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("fixture dir");
        std::fs::write(path, text).expect("fixture file");
    }

    fn names(&self) -> Vec<String> {
        check(&self.root)
            .expect("the fixture is readable")
            .into_iter()
            .map(|finding| finding.name)
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

#[test]
fn the_repository_passes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let findings = check(&root).expect("the Aviate tree is readable");
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn exemptions_in_their_own_files_pass() {
    assert!(Fixture::new("clean").names().is_empty());
}

#[test]
fn every_response_constant_outside_the_exemptions_is_named() {
    for (path, declaration_text, expected) in [
        (
            "uplink/feel.rs",
            "const MAX_TAKEOFF_THRUST: f32 = 0.75;",
            "MAX_TAKEOFF_THRUST",
        ),
        (
            "adapter/flight.rs",
            "const MAX_ROLL_RATE: f32 = 0.6;",
            "MAX_ROLL_RATE",
        ),
        (
            "adapter/flight.rs",
            "const RESPONSE_CURVE: [f32; 2] = [0.2, 0.8];",
            "RESPONSE_CURVE",
        ),
        (
            "adapter/flight.rs",
            "const MAX_DT_S: f32 = 0.1;",
            "MAX_DT_S",
        ),
        (
            "adapter/flight.rs",
            "static RELEASE_DWELL: std::time::Duration = std::time::Duration::from_millis(20);",
            "RELEASE_DWELL",
        ),
        (
            "adapter/flight.rs",
            "const NEUTRAL_DWELL_MS: u32 = 20;",
            "NEUTRAL_DWELL_MS",
        ),
        ("adapter/flight.rs", "const MAGIC: f32 = 0.4;", "MAGIC"),
        (
            "adapter/flight.rs",
            "const COMMAND_SETTLE: std::time::Duration = std::time::Duration::from_millis(20);",
            "COMMAND_SETTLE",
        ),
        (
            "adapter/flight.rs",
            "const COMMAND_SETTLE_MS: u32 = 20;",
            "COMMAND_SETTLE_MS",
        ),
        (
            "adapter/flight.rs",
            "const APPLY_STEPS: u32 = 4;",
            "APPLY_STEPS",
        ),
        (
            "adapter/flight.rs",
            "const REVERSAL_STEPS: u32 = 4;",
            "REVERSAL_STEPS",
        ),
        (
            "uplink/feel.rs",
            "const HOLD_KEEPALIVE: Duration = Duration::ZERO;",
            "HOLD_KEEPALIVE",
        ),
    ] {
        let fixture = Fixture::new("reject");
        fixture.write(path, &format!("{declaration_text}\n"));
        assert_eq!(
            fixture.names(),
            vec![expected.to_owned()],
            "{declaration_text}"
        );
    }
}

#[test]
fn comments_and_non_response_types_pass() {
    let fixture = Fixture::new("comment");
    fixture.write(
        "adapter/flight.rs",
        "// const RESPONSE_LIMIT: f32 = 0.4;\nconst MAGIC: DurationPolicy = DurationPolicy::Fixed;\n",
    );
    assert!(fixture.names().is_empty());
}

#[test]
fn an_exemption_is_used_once() {
    let fixture = Fixture::new("reuse");
    fixture.write(
        "uplink.rs",
        "const MAX_DT_S: f32 = 0.1;\nmod nested {\n    const MAX_DT_S: f32 = 0.2;\n}\n",
    );
    assert_eq!(fixture.names(), vec!["MAX_DT_S".to_owned()]);
}

#[test]
fn a_public_profile_mutation_on_the_uplink_is_refused() {
    for entry in [
        "pub fn new_with_profile(profile: ValidatedFlightFeelProfile) {}",
        "pub fn install_profile(profile: ValidatedFlightFeelProfile) {}",
        "pub const fn install_profile (profile: ValidatedFlightFeelProfile) {}",
    ] {
        let fixture = Fixture::new("entry");
        fixture.write(
            "uplink.rs",
            &format!("const MAX_DT_S: f32 = 0.1;\n{entry}\n"),
        );
        assert_eq!(
            fixture.names(),
            vec!["custom-profile-entry".to_owned()],
            "{entry}"
        );
    }
    let fixture = Fixture::new("crate-entry");
    fixture.write(
        "uplink.rs",
        "const MAX_DT_S: f32 = 0.1;\npub(crate) fn new_with_profile(p: P) {}\n",
    );
    assert!(
        fixture.names().is_empty(),
        "a crate-private entry is not public"
    );
}

#[test]
fn declarations_parse_their_name_and_type() {
    assert_eq!(
        declaration("    pub(crate) const MAX_DT_S: f32 = 0.1;"),
        Some(("MAX_DT_S", " f32 "))
    );
    assert_eq!(declaration("static lower: f32 = 0.1;"), None);
    assert_eq!(declaration("constant MAX: f32 = 0.1;"), None);
    assert!(reads_as_response("SOMETHING_MS", " u32 "));
    assert!(!reads_as_response("ROLLER", " u32 "));
}
