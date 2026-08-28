#![allow(clippy::expect_used, clippy::panic)]

use super::discover_pairs;

/// Discovery is by convention: a check with a self-test is a pair, a
/// check without one is not, and nothing else qualifies.
#[test]
fn discovery_pairs_checks_with_their_self_tests() {
    let scripts =
        std::env::temp_dir().join(format!("xtask-guards-discovery-{}", std::process::id()));
    std::fs::create_dir_all(&scripts).expect("create scratch scripts dir");
    for file in [
        "check-paired.sh",
        "test-check-paired.sh",
        "check-unpaired.sh",
        "test-check-orphan-self-test.sh",
        "unrelated.sh",
    ] {
        std::fs::write(scripts.join(file), "#!/bin/sh\n").expect("write script");
    }

    let pairs = discover_pairs(&scripts).expect("discover");
    let names: Vec<&str> = pairs.iter().map(|pair| pair.name.as_str()).collect();
    assert_eq!(names, vec!["paired"]);

    std::fs::remove_dir_all(&scripts).ok();
}

/// Every pair CI relies on is pinned by name. Discovery wires a guard
/// into CI only while its self-test exists, so deleting a self-test
/// would silently drop its guard from CI; this floor turns that into
/// a loud failure. Migrating a guard to Rust deletes its pair and must
/// remove its name here in the same change.
#[test]
fn repository_guards_are_discovered() {
    let scripts = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts")
        .canonicalize()
        .expect("scripts directory");
    let pairs = discover_pairs(&scripts).expect("discover");
    let names: Vec<&str> = pairs.iter().map(|pair| pair.name.as_str()).collect();
    for expected in [
        "adr-supersession",
        "airspace-view-contract",
        "apple-client",
        "aviate-control-feel-boundary",
        "aviate-supervision-boundary",
        "flight-build",
        "flight-tune-boundaries",
        "flight-tune-journal-storage-boundary",
        "geodetic-datum-codes",
        "monotonic-counter-arithmetic",
        "navdata-tile-bundle",
        "production-rust-lints",
        "situation-layer-controls",
        "situation-map-attribution",
        "situation-ownship-wiring",
        "situation-presentation-boundary",
        "structure",
        "terrain-base-layer",
        "web-situation-map",
    ] {
        assert!(
            names.contains(&expected),
            "{expected} missing from {names:?}"
        );
    }
}
