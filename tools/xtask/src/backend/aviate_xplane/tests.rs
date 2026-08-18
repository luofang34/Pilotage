//! Aviate X-Plane backend: profile refusal, the host environment the
//! payload view needs, and the simulator-ownership discipline.

#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use super::{AIRFRAME, AviateXPlane};
use crate::backend::{SessionContext, SimBackend};
use crate::cli::Profile;
use crate::error::XtaskError;

fn context(profile: Profile) -> SessionContext {
    SessionContext {
        repo_root: PathBuf::from("/repo"),
        host_port: 4433,
        viewer_port: 8080,
        profile,
        log_dir: std::env::temp_dir(),
        lan: false,
    }
}

#[test]
fn plan_refuses_physical_and_oracle_only_profiles() {
    for profile in [Profile::Physical, Profile::OracleOnly] {
        let refusal = AviateXPlane.plan(&context(profile));
        assert!(
            matches!(refusal, Err(XtaskError::Usage { .. })),
            "{profile:?} must be refused, got {refusal:?}"
        );
    }
}

#[test]
fn the_host_sources_the_payload_view_from_the_simulator_plugin() {
    let env = AviateXPlane.host_env(&context(Profile::Simulation));
    assert!(
        env.iter()
            .any(|(key, value)| key == "PILOTAGE_AVIATE_CAMERA" && value == "xplane-plugin"),
        "the gimbal scope is only real when a producer accepts commands"
    );
    assert!(
        env.iter()
            .any(|(key, value)| key == "PILOTAGE_AVIATE_PROFILE" && value == "simulation")
    );
    assert!(env.iter().any(|(key, value)| {
        key == "PILOTAGE_RESET_CMD" && value.ends_with("scripts/reset-xplane-sim.sh")
    }));
}

#[test]
fn stale_patterns_name_the_flight_controller_not_the_simulator() {
    let patterns = AviateXPlane.stale_process_patterns();
    assert!(
        patterns
            .iter()
            .any(|pattern| pattern.contains("sitl-xplane")),
        "a stale flight controller must be refused"
    );
    for pattern in patterns {
        assert!(
            !pattern.contains("X-Plane"),
            "the launcher must never kill the operator's simulator, got {pattern:?}"
        );
    }
}

#[test]
fn the_backend_pins_an_airframe_the_flight_controller_can_mix() {
    // The flight controller mixes four rotors in a quad-X; the pinned
    // airframe's channel map must be the matching one, or the commands
    // would drive the wrong surfaces.
    assert_eq!(AIRFRAME, "alia250");
    assert!(
        crate::backend::xplane_simulator::airframe_for(Some(AIRFRAME)).is_ok(),
        "the pinned airframe must exist in the simulator's table"
    );
}
