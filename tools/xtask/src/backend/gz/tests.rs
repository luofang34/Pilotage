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

/// Every gz flight deck carries one origin and one spawn frame, and
/// relocating moves exactly those: the origin to the point and its
/// elevation, the frame to the point's heading.
#[test]
fn every_gz_world_relocates_to_the_spawn_point() {
    use super::relocate_world;
    use crate::backend::SpawnPoint;

    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root");
    let point = SpawnPoint {
        latitude_deg: 40.577_186_1,
        longitude_deg: -74.738_191_8,
        elevation_m: 56.3,
        heading_deg: 30.0,
    };
    for world in [
        "sim/worlds/px4_flightdeck.sdf",
        "sim/worlds/px4_rc_cessna_flightdeck.sdf",
        "sim/worlds/x500_flightdeck.sdf",
    ] {
        let source = std::fs::read_to_string(repo_root.join(world)).expect("world");
        let moved = relocate_world(&source, &point).expect(world);
        for expected in [
            "<latitude_deg>40.577186100</latitude_deg>",
            "<longitude_deg>-74.738191800</longitude_deg>",
            "<elevation>56.300000000</elevation>",
            "<frame name=\"vehicle_spawn\">\n      <pose>0 0 0 0 0 1.0471976</pose>",
        ] {
            assert!(moved.contains(expected), "{world}: {expected}");
        }
        assert_eq!(
            super::world_elevation(&moved),
            Ok(56.3),
            "{world}: the FC reads the site elevation back"
        );
        let unchanged = |text: &str| -> Vec<String> {
            text.lines()
                .filter(|line| {
                    ![
                        "latitude_deg",
                        "longitude_deg",
                        "<elevation>",
                        "<pose>0 0 0 0 0 ",
                    ]
                    .iter()
                    .any(|tag| line.contains(tag))
                })
                .map(str::to_owned)
                .collect()
        };
        assert_eq!(
            unchanged(&moved),
            unchanged(&source),
            "{world}: only the origin and the spawn frame move"
        );
    }
}

/// The vehicle and its camera rig are posed in the spawn frame, so the
/// frame's yaw turns both; a vehicle posed anywhere else would keep
/// facing the authored way.
#[test]
fn every_gz_vehicle_and_its_rig_stand_in_the_spawn_frame() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root");
    for world in [
        "sim/worlds/px4_flightdeck.sdf",
        "sim/worlds/px4_rc_cessna_flightdeck.sdf",
        "sim/worlds/x500_flightdeck.sdf",
    ] {
        let source = std::fs::read_to_string(repo_root.join(world)).expect("world");
        assert_eq!(
            source
                .matches("<pose relative_to=\"vehicle_spawn\">")
                .count(),
            2,
            "{world}: the vehicle include and the camera rig"
        );
        let rig = source.find("_camera_rig\">").expect("rig model");
        assert!(
            source[rig..]
                .trim_start_matches("_camera_rig\">")
                .trim_start()
                .starts_with("<pose relative_to=\"vehicle_spawn\">"),
            "{world}: the rig's own pose is in the spawn frame"
        );
    }
}

/// A true heading is clockwise from north; gz's ENU yaw is
/// counter-clockwise from east. Runway 04's 030° is 60° in ENU.
#[test]
fn true_headings_become_enu_yaw() {
    use super::enu_yaw_rad;
    let close =
        |heading: f64, yaw_deg: f64| (enu_yaw_rad(heading) - yaw_deg.to_radians()).abs() < 1e-12;
    assert!(close(30.0, 60.0));
    assert!(close(0.0, 90.0));
    assert!(close(90.0, 0.0));
    assert!(close(180.0, -90.0));
    assert!(close(270.0, 180.0));
    assert!(close(300.0, 150.0));
}

#[test]
fn a_world_without_one_origin_or_spawn_frame_is_refused() {
    use super::relocate_world;
    use crate::backend::SpawnPoint;

    let point = SpawnPoint {
        latitude_deg: 1.0,
        longitude_deg: 2.0,
        elevation_m: 3.0,
        heading_deg: 4.0,
    };
    assert_eq!(relocate_world("<sdf/>", &point), Err("latitude_deg"));
    let twice = "<latitude_deg>0</latitude_deg><latitude_deg>0</latitude_deg>";
    assert_eq!(relocate_world(twice, &point), Err("latitude_deg"));
    let no_longitude = "<latitude_deg>0</latitude_deg>";
    assert_eq!(relocate_world(no_longitude, &point), Err("longitude_deg"));
    let origin = "<latitude_deg>0</latitude_deg><longitude_deg>0</longitude_deg>";
    assert_eq!(relocate_world(origin, &point), Err("elevation"));
    let no_frame = format!("{origin}<elevation>0</elevation>");
    assert_eq!(relocate_world(&no_frame, &point), Err("vehicle_spawn"));
    let frame = "<frame name=\"vehicle_spawn\"><pose>0 0 0 0 0 0</pose></frame>";
    let two_frames = format!("{no_frame}{frame}{frame}");
    assert_eq!(relocate_world(&two_frames, &point), Err("vehicle_spawn"));
    let poseless = format!("{no_frame}<frame name=\"vehicle_spawn\"></frame><pose>0</pose>");
    assert_eq!(relocate_world(&poseless, &point), Err("vehicle_spawn"));
    let one = relocate_world(&format!("{no_frame}{frame}"), &point).expect("relocates");
    assert!(
        one.ends_with("<frame name=\"vehicle_spawn\"><pose>0 0 0 0 0 1.5009832</pose></frame>")
    );
}
