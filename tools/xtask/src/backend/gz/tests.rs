//! Partition tests: instance 0 keeps the default partition, every other
//! session gets one no other backend or instance shares.

#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use super::{partition, partition_claims, partition_env};
use crate::backend::SessionContext;
use crate::conflicts::Claim;

fn context(instance: u16) -> SessionContext {
    let mut ctx = SessionContext::for_test(PathBuf::from("unused"));
    ctx.instance = instance;
    ctx
}

#[test]
fn instance_zero_keeps_the_default_partition_and_claims_it() {
    let ctx = context(0);
    assert_eq!(partition(&ctx, "px4-gz"), None);
    assert!(partition_env(&ctx, "px4-gz").is_empty());
    assert_eq!(
        partition_claims(&ctx, "px4-gz"),
        vec![Claim::DefaultGzPartition]
    );
}

#[test]
fn every_other_session_gets_a_partition_of_its_own() {
    let five = partition(&context(5), "px4-gz").expect("instance 5 is partitioned");
    let six = partition(&context(6), "px4-gz").expect("instance 6 is partitioned");
    let other_backend = partition(&context(5), "aviate-gz").expect("partitioned");
    assert_ne!(five, six, "instances never share a partition");
    assert_ne!(five, other_backend, "backends never share a partition");
    assert_eq!(
        partition_env(&context(5), "px4-gz"),
        vec![("GZ_PARTITION".to_owned(), five)]
    );
    assert!(partition_claims(&context(5), "px4-gz").is_empty());
}

/// Both gz flight decks carry one origin, and relocating moves exactly
/// that origin and nothing else.
#[test]
fn every_gz_world_relocates_to_the_spawn_point() {
    use super::relocate_world;
    use crate::backend::SpawnPoint;

    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root");
    let point = SpawnPoint {
        latitude_deg: 47.397742,
        longitude_deg: 8.545999,
    };
    for world in [
        "sim/worlds/px4_flightdeck.sdf",
        "sim/worlds/px4_rc_cessna_flightdeck.sdf",
        "sim/worlds/x500_flightdeck.sdf",
    ] {
        let source = std::fs::read_to_string(repo_root.join(world)).expect("world");
        let moved = relocate_world(&source, &point).expect(world);
        assert!(
            moved.contains("<latitude_deg>47.397742000</latitude_deg>"),
            "{world}"
        );
        assert!(
            moved.contains("<longitude_deg>8.545999000</longitude_deg>"),
            "{world}"
        );
        let unchanged = |text: &str| -> Vec<String> {
            text.lines()
                .filter(|line| !line.contains("latitude_deg") && !line.contains("longitude_deg"))
                .map(str::to_owned)
                .collect()
        };
        assert_eq!(
            unchanged(&moved),
            unchanged(&source),
            "{world}: only the origin moves"
        );
    }
}

#[test]
fn a_world_without_one_origin_is_refused() {
    use super::relocate_world;
    use crate::backend::SpawnPoint;

    let point = SpawnPoint {
        latitude_deg: 1.0,
        longitude_deg: 2.0,
    };
    assert_eq!(relocate_world("<sdf/>", &point), Err("latitude_deg"));
    let twice = "<latitude_deg>0</latitude_deg><latitude_deg>0</latitude_deg>";
    assert_eq!(relocate_world(twice, &point), Err("latitude_deg"));
    let no_longitude = "<latitude_deg>0</latitude_deg>";
    assert_eq!(relocate_world(no_longitude, &point), Err("longitude_deg"));
}
