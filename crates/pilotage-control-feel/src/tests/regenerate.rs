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
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapters/aviate/profiles");
        // The Alia keeps the law as written; the x500 gets the same law fitted
        // to the tilt its velocity loop may command.
        for limits in [
            crate::airframe::AirframeLimits::ALIA250,
            crate::airframe::AirframeLimits::X500,
        ] {
            let profile = FlightFeelProfile::shaped_for(limits, mode);
            let json = serde_json::to_string(&profile).expect("encode the shaped profile");
            let path = dir.join(format!("{}-shaped-{name}-v1.json", limits.id));
            std::fs::write(path, json + "\n").expect("write the shipped profile");
        }
    }
}
