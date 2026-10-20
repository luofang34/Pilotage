#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use super::{SIM_CAMERA_PORT_BASE, host_env, port, stage};
use crate::backend::SessionContext;
use crate::readiness::Readiness;

fn context(instance: u16) -> SessionContext {
    SessionContext {
        instance,
        sim_camera: true,
        ..SessionContext::for_test(PathBuf::from("/repo"))
    }
}

#[test]
fn each_instance_has_its_own_camera_port() {
    assert_eq!(port(&context(0)).expect("port"), SIM_CAMERA_PORT_BASE);
    assert_eq!(port(&context(7)).expect("port"), SIM_CAMERA_PORT_BASE + 7);
    assert!(port(&context(u16::MAX)).is_err(), "no port past 65535");
}

#[test]
fn the_producer_dials_the_port_the_host_binds() {
    let ctx = context(3);
    let stage = stage(&ctx, None).expect("stage");
    let host_port = host_env(port(&ctx).expect("port"))
        .into_iter()
        .find(|(name, _)| name == "PILOTAGE_SIM_CAMERA_PORT")
        .map(|(_, value)| value)
        .expect("the host binds a port");
    let dialed = stage
        .spec
        .args
        .windows(2)
        .find(|pair| pair[0] == "--host-port")
        .map(|pair| pair[1].clone())
        .expect("the producer dials a port");
    assert_eq!(dialed, host_port);
    assert!(stage.spec.program.ends_with("pilotage-maplibre-camera"));
    assert!(matches!(stage.readiness, Readiness::LogContains { .. }));
}

#[test]
fn the_host_turns_every_adapter_camera_off() {
    let env = host_env(46_000);
    for name in ["PILOTAGE_PX4_CAMERA", "PILOTAGE_AVIATE_CAMERA"] {
        assert!(
            env.iter().any(|(key, value)| key == name && value == "off"),
            "{name} must be off: the host refuses a second producer"
        );
    }
}

#[test]
fn every_backend_host_binds_the_camera_port_only_with_the_stage() {
    for entry in crate::backend::BACKENDS {
        let backend = crate::backend::backend_for(entry.name).expect("registered");
        let with = crate::session::host_stage(&context(4), backend.as_ref()).expect("host stage");
        let bound = with
            .spec
            .env
            .iter()
            .find(|(name, _)| name == "PILOTAGE_SIM_CAMERA_PORT")
            .map(|(_, value)| value.clone());
        assert_eq!(
            bound,
            Some((SIM_CAMERA_PORT_BASE + 4).to_string()),
            "{}",
            entry.name
        );
        let without = crate::session::host_stage(
            &SessionContext::for_test(PathBuf::from("/repo")),
            backend.as_ref(),
        )
        .expect("host stage");
        assert!(
            !without
                .spec
                .env
                .iter()
                .any(|(name, _)| name == "PILOTAGE_SIM_CAMERA_PORT"),
            "{} binds a camera port nobody asked for",
            entry.name
        );
    }
}

#[test]
fn a_known_spawn_point_is_cached_before_the_cameras_start() {
    let with = stage(&context(1), Some((40.5771861, -74.7381918))).expect("stage");
    let prewarm = with
        .spec
        .args
        .windows(2)
        .find(|pair| pair[0] == "--prewarm")
        .map(|pair| pair[1].clone());
    assert_eq!(prewarm.as_deref(), Some("40.5771861,-74.7381918"));
    let without = stage(&context(1), None).expect("stage");
    assert!(!without.spec.args.iter().any(|arg| arg == "--prewarm"));
}

/// The adapter camera each backend's host is handed, one value per
/// variable.
fn adapter_cameras(env: &[(String, String)]) -> Vec<(&str, &str)> {
    env.iter()
        .filter(|(name, _)| name == "PILOTAGE_PX4_CAMERA" || name == "PILOTAGE_AVIATE_CAMERA")
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

/// Only a session that runs Gazebo spawns the gz camera sidecar, and
/// every JSBSim session says so itself rather than leaning on an adapter
/// default; with the world-model camera stage every adapter camera is
/// off, each named once.
#[test]
fn only_gazebo_sessions_select_the_gz_camera() {
    for entry in crate::backend::BACKENDS {
        let backend = crate::backend::backend_for(entry.name).expect("registered");
        let plain = SessionContext::for_test(PathBuf::from("/repo"));
        let without = crate::session::host_stage(&plain, backend.as_ref()).expect("host stage");
        let cameras = adapter_cameras(&without.spec.env);
        let gazebo = cameras.iter().any(|(_, value)| *value == "gazebo");
        assert_eq!(
            gazebo,
            entry.name.ends_with("-gz"),
            "{} selects {cameras:?}",
            entry.name
        );
        if entry.name.contains("jsbsim") {
            assert!(
                !cameras.is_empty() && cameras.iter().all(|(_, value)| *value == "off"),
                "{} must state its camera off, got {cameras:?}",
                entry.name
            );
        }
        let with = crate::session::host_stage(&context(4), backend.as_ref()).expect("host stage");
        let mut cameras = adapter_cameras(&with.spec.env);
        cameras.sort_unstable();
        assert_eq!(
            cameras,
            [
                ("PILOTAGE_AVIATE_CAMERA", "off"),
                ("PILOTAGE_PX4_CAMERA", "off")
            ],
            "{}",
            entry.name
        );
    }
}
