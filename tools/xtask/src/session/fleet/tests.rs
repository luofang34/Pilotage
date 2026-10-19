//! Fleet planning tests: distinct names, ports, instances, and spawn
//! points per vehicle on the site, self-conflicts refused up front, and a
//! manifest whose entries carry the single-session connect fields.

#![allow(clippy::expect_used, clippy::panic)]

use std::path::Path;

use super::{
    FleetEntry, FleetManifest, refuse_overlapping_claims, resolve_slots, session_name, spawn_points,
};
use crate::cli::{FleetArgs, Profile};
use crate::conflicts::{Claim, session_claims};
use crate::error::XtaskError;
use crate::site::{DEFAULT_SITE, Ground};

fn fleet(vehicles: &[&str]) -> FleetArgs {
    FleetArgs {
        vehicles: vehicles.iter().map(|v| (*v).to_owned()).collect(),
        profile: Profile::Simulation,
        host_port: 4533,
        viewer_port: 8180,
        instance_base: 5,
        open: false,
        site: DEFAULT_SITE,
        ready_timeout_s: crate::backend::DEFAULT_READY_TIMEOUT_S,
        sim_camera: false,
    }
}

/// Metres between two points on a sphere, flat at the default site's
/// latitude: within half a metre over a layout.
fn distance_m(a: &crate::backend::SpawnPoint, b: &crate::backend::SpawnPoint) -> f64 {
    let north = (b.latitude_deg - a.latitude_deg) * 111_320.0;
    let east = (b.longitude_deg - a.longitude_deg)
        * 111_320.0
        * DEFAULT_SITE.latitude_deg.to_radians().cos();
    north.hypot(east)
}

/// The plane takes the runway threshold wherever it is in the launch
/// order; the quads stand in a row on the grass, 40 m apart, and no two
/// vehicles share a spot.
#[test]
fn the_plane_takes_the_threshold_and_the_quads_a_row_beside_the_runway() {
    let slots = resolve_slots(
        &fleet(&[
            "px4-gz:x500_gimbal",
            "ardupilot-sitl",
            "px4-gz:rc_cessna",
            "betaflight-sitl",
        ]),
        Path::new("/repo"),
    )
    .expect("slots resolve");
    let grounds: Vec<Ground> = slots.iter().map(|slot| slot.backend.ground()).collect();
    assert_eq!(
        grounds,
        [Ground::Field, Ground::Field, Ground::Runway, Ground::Field]
    );
    let points = spawn_points(&DEFAULT_SITE, &grounds);
    let plane = points[2];
    assert_eq!(
        (plane.latitude_deg, plane.longitude_deg),
        (DEFAULT_SITE.latitude_deg, DEFAULT_SITE.longitude_deg)
    );
    assert_eq!(plane.heading_deg, DEFAULT_SITE.heading_deg);
    for (first, second) in [(0, 1), (1, 3)] {
        let step = distance_m(&points[first], &points[second]);
        assert!((step - 40.0).abs() < 0.5, "{first}->{second}: {step} m");
    }
    for (index, point) in points.iter().enumerate() {
        assert_eq!(
            point.elevation_m, DEFAULT_SITE.elevation_m,
            "vehicle {index}"
        );
        for other in points.iter().skip(index + 1) {
            assert!(distance_m(point, other) > 30.0, "vehicle {index} overlaps");
        }
    }
}

#[test]
fn each_vehicle_gets_its_own_name_ports_instance_and_run_directory() {
    let repo = Path::new("/repo");
    let slots = resolve_slots(
        &fleet(&["px4-gz:x500_gimbal", "px4-gz:rc_cessna", "px4-gz"]),
        repo,
    )
    .expect("slots resolve");
    let names: Vec<&str> = slots.iter().map(|slot| slot.name.as_str()).collect();
    assert_eq!(
        names,
        ["px4-gz-x500_gimbal-1", "px4-gz-rc_cessna-2", "px4-gz-3"]
    );
    for (index, slot) in slots.iter().enumerate() {
        let offset = u16::try_from(index).expect("small");
        assert_eq!(slot.ctx.host_port, 4533 + offset);
        assert_eq!(slot.ctx.viewer_port, 8180 + offset);
        assert_eq!(slot.ctx.instance, 5 + offset);
        assert_eq!(
            slot.ctx.log_dir,
            repo.join("target/xtask-fleet").join(&slot.name)
        );
    }
    assert_eq!(session_name("aviate-gz", 0), "aviate-gz-1");
}

#[test]
fn an_unknown_vehicle_fails_before_anything_starts() {
    let refusal = resolve_slots(&fleet(&["px4-gz", "px4-gz:x501"]), Path::new("/repo"));
    assert!(matches!(refusal, Err(XtaskError::UnknownModel { .. })));
}

/// Two Aviate quads would share the FC's shared-memory block and its
/// telemetry port; the fleet refuses that itself, before probing.
#[test]
fn a_fleet_that_conflicts_with_itself_is_refused_naming_both_vehicles() {
    let slots = resolve_slots(
        &fleet(&["aviate-gz", "px4-gz", "aviate-gz"]),
        Path::new("/repo"),
    )
    .expect("slots resolve");
    let claims: Vec<(String, Vec<Claim>)> = slots
        .iter()
        .map(|slot| {
            (
                slot.name.clone(),
                session_claims(&slot.ctx, slot.backend.as_ref()),
            )
        })
        .collect();
    let Err(XtaskError::FleetClaimOverlap { first, second, .. }) =
        refuse_overlapping_claims(&claims)
    else {
        panic!("two aviate-gz vehicles must be refused");
    };
    assert_eq!(
        (first.as_str(), second.as_str()),
        ("aviate-gz-1", "aviate-gz-3")
    );

    refuse_overlapping_claims(&claims[..2]).expect("aviate-gz beside px4-gz shares nothing");
}

/// Each entry carries exactly the connect fields of the single-session
/// `session.json` (host, port, certHash), plus the vehicle's name and
/// backend.
#[test]
fn the_fleet_manifest_carries_the_session_manifest_fields_per_vehicle() {
    let cert = "a".repeat(64);
    let manifest = FleetManifest {
        sessions: vec![FleetEntry {
            name: "px4-gz-rc_cessna-2".to_owned(),
            backend: "px4-gz:rc_cessna".to_owned(),
            host: "127.0.0.1".to_owned(),
            port: 4534,
            cert_hash: cert.clone(),
        }],
    };
    let value = serde_json::to_value(&manifest).expect("serializes");
    let single: serde_json::Value = serde_json::from_str(&crate::readiness::session_manifest(
        "127.0.0.1",
        None,
        4534,
        &cert,
    ))
    .expect("session manifest parses");
    let entry = &value["sessions"][0];
    for (key, expected) in single.as_object().expect("object") {
        assert_eq!(&entry[key], expected, "{key}");
    }
    assert_eq!(entry["name"], "px4-gz-rc_cessna-2");
    assert_eq!(entry["backend"], "px4-gz:rc_cessna");
    assert_eq!(entry.as_object().expect("object").len(), 5);
}
