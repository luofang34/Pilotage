#![allow(clippy::expect_used, clippy::panic)]

use std::env::VarError;
use std::ffi::OsString;
use std::net::SocketAddr;

use super::{Px4Env, config_from_env};
use crate::error::HostError;

fn absent() -> Result<String, VarError> {
    Err(VarError::NotPresent)
}

fn set(value: &str) -> Result<String, VarError> {
    Ok(value.to_owned())
}

/// A simulation-profile environment with every optional variable unset.
fn simulation() -> Px4Env {
    Px4Env {
        profile: set("simulation"),
        telemetry_endpoint: absent(),
        stream_command_endpoint: absent(),
        command_endpoint: absent(),
        system_id: absent(),
        gimbal: absent(),
        drop_gimbal_stop: absent(),
    }
}

fn config(profile: Result<String, VarError>) -> Result<pilotage_adapter_px4::Px4Config, HostError> {
    config_from_env(Px4Env {
        profile,
        ..simulation()
    })
}

#[test]
fn direct_px4_start_requires_an_explicit_simulation_profile() {
    assert!(matches!(
        config(absent()),
        Err(HostError::Px4ProfileMissing)
    ));
    let accepted = config(set("simulation")).expect("simulation profile");
    assert_eq!(
        accepted.telemetry_endpoint,
        SocketAddr::from(([127, 0, 0, 1], 14_550))
    );
}

#[test]
fn direct_px4_start_refuses_every_non_simulation_profile() {
    for value in ["physical", "oracle-only", "simulaton", "Simulation", ""] {
        let refusal = config(set(value));
        assert!(
            matches!(refusal, Err(HostError::Px4Profile { value: ref rejected }) if rejected == value),
            "{value:?} must be refused, got {refusal:?}"
        );
    }
}

#[test]
fn direct_px4_start_refuses_non_unicode_profile_values() {
    #[cfg(unix)]
    let raw = {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(vec![0x66, 0xFF, 0x6F])
    };
    #[cfg(not(unix))]
    let raw = OsString::from("\u{FFFD}garbage");
    let refusal = config(Err(VarError::NotUnicode(raw)));
    assert!(matches!(refusal, Err(HostError::Px4ProfileEncoding { .. })));
}

#[test]
fn present_invalid_endpoints_fail_instead_of_using_defaults() {
    for (variable, refusal) in [
        (
            "PILOTAGE_PX4_ADDR",
            config_from_env(Px4Env {
                telemetry_endpoint: set("not-an-address"),
                ..simulation()
            }),
        ),
        (
            "PILOTAGE_PX4_GCS_ADDR",
            config_from_env(Px4Env {
                stream_command_endpoint: set("127.0.0.1:not-a-port"),
                ..simulation()
            }),
        ),
        (
            "PILOTAGE_PX4_FC_ADDR",
            config_from_env(Px4Env {
                command_endpoint: set("missing-port"),
                ..simulation()
            }),
        ),
    ] {
        assert!(
            matches!(refusal, Err(HostError::Px4Endpoint { variable: rejected, .. }) if rejected == variable),
            "{variable} must fail closed, got {refusal:?}"
        );
    }
}

#[test]
fn the_system_id_defaults_to_one_and_accepts_a_sitl_instance_id() {
    assert_eq!(config_from_env(simulation()).expect("default").system_id, 1);
    let seventh = config_from_env(Px4Env {
        system_id: set("7"),
        ..simulation()
    })
    .expect("instance 6");
    assert_eq!(seventh.system_id, 7);
}

#[test]
fn a_broadcast_or_unparsable_system_id_fails_closed() {
    for value in ["0", "256", "-1", "seven", ""] {
        let refusal = config_from_env(Px4Env {
            system_id: set(value),
            ..simulation()
        });
        assert!(
            matches!(
                refusal,
                Err(HostError::Px4Setting {
                    variable: "PILOTAGE_PX4_SYSTEM_ID",
                    ..
                })
            ),
            "{value:?} must be refused, got {refusal:?}"
        );
    }
}
