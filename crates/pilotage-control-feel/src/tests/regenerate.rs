//! Writes the shipped profile artifacts from the code that shapes them.
//!
//! Ignored by default: it is a generator, not a check. The check that the
//! files match lives beside the adapter that installs them.

#![allow(clippy::expect_used, clippy::panic)]

use crate::FlightFeelProfile;

#[test]
#[ignore = "regenerates the shipped profile artifacts; run explicitly"]
fn regenerate_shipped_profiles() {
    for (mode, name) in [
        (crate::FeelMode::Precision, "precision"),
        (crate::FeelMode::Balanced, "balanced"),
        (crate::FeelMode::Agile, "agile"),
    ] {
        let profile = FlightFeelProfile::shaped(mode);
        let json = serde_json::to_string(&profile).expect("encode the shaped profile");
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../adapters/aviate/profiles")
            .join(format!("alia250-shaped-{name}-v1.json"));
        std::fs::write(path, json + "\n").expect("write the shipped profile");
    }
}
