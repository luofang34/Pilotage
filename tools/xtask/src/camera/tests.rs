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
    let stage = stage(&ctx).expect("stage");
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
