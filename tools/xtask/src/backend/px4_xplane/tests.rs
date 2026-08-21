//! Plan and host-environment tests for the px4-xplane backend: profile
//! refusal, fail-closed airframe selection, install validation with
//! actionable hints, root discovery, and the CMND wire shape.

#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use super::super::xplane_simulator::{
    Airframe, airframe_for, command_datagram, set_active_config_name, validate_xplane_install,
    xplane_root_from,
};
use super::Px4XPlane;
use crate::backend::{SessionContext, SimBackend};
use crate::cli::Profile;
use crate::error::XtaskError;

fn context(repo_root: PathBuf) -> SessionContext {
    SessionContext {
        repo_root,
        host_port: 4433,
        viewer_port: 8080,
        profile: Profile::Simulation,
        log_dir: std::env::temp_dir(),
        lan: false,
    }
}

fn scaffold(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("px4-xplane-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("scaffold");
    root
}

fn qtailsitter() -> &'static Airframe {
    airframe_for(None).expect("default airframe")
}

#[test]
fn plan_refuses_physical_and_oracle_only_profiles() {
    let backend = Px4XPlane;
    for profile in [Profile::Physical, Profile::OracleOnly] {
        let mut ctx = context(PathBuf::from("unused-for-profile-refusal"));
        ctx.profile = profile;
        let refusal = backend.plan(&ctx);
        assert!(
            matches!(refusal, Err(XtaskError::Usage { .. })),
            "{profile:?} must be refused, got {refusal:?}"
        );
    }
}

#[test]
fn airframe_selection_defaults_and_fails_closed() {
    assert_eq!(airframe_for(None).expect("default").key, "qtailsitter");
    assert_eq!(
        airframe_for(Some("cessna172"))
            .expect("stock")
            .sys_autostart,
        "5001"
    );
    assert_eq!(
        airframe_for(Some("alia250")).expect("stock").sys_autostart,
        "5020"
    );
    let refusal = airframe_for(Some("ehang184"));
    assert!(
        matches!(refusal, Err(XtaskError::Usage { .. })),
        "an aircraft this machine cannot fly must be refused, got {refusal:?}"
    );
}

#[test]
fn host_environment_declares_profile_and_reset_but_no_gimbal() {
    let backend = Px4XPlane;
    let ctx = context(PathBuf::from("/repo"));
    let env = backend.host_env(&ctx);
    assert!(
        env.iter()
            .any(|(key, value)| key == "PILOTAGE_PX4_PROFILE" && value == "simulation")
    );
    assert!(
        env.iter().any(|(key, value)| {
            key == "PILOTAGE_RESET_CMD" && value.ends_with("scripts/reset-xplane-sim.sh")
        }),
        "the viewer reset must route to the X-Plane reset script"
    );
    // No gimbal device exists in the X-Plane bridge; advertising the
    // scope would offer a control surface with no enactment behind it.
    assert!(env.iter().all(|(key, _)| key != "PILOTAGE_PX4_GIMBAL"));
    // The vehicle view comes from the in-simulator camera plugin, which
    // dials the host: no producer binary path belongs in the host's
    // environment.
    assert!(
        env.iter()
            .any(|(key, value)| key == "PILOTAGE_PX4_CAMERA" && value == "xplane-plugin")
    );
    assert!(env.iter().all(|(key, _)| key != "PILOTAGE_SIM_VIDEO_BIN"));
}

#[test]
fn stale_patterns_never_name_the_operator_owned_simulator() {
    let backend = Px4XPlane;
    for pattern in backend.stale_process_patterns() {
        assert!(
            !pattern.contains("X-Plane"),
            "the launcher must never kill X-Plane, got pattern {pattern:?}"
        );
    }
}

#[test]
fn install_validation_hints_at_the_build_script() {
    let root = scaffold("install-validation");
    let refusal = validate_xplane_install(&root, qtailsitter());
    match refusal {
        Err(XtaskError::MissingArtifact { what, hint, .. }) => {
            assert_eq!(what, "px4xplane bridge plugin");
            assert!(hint.contains("build-xplane-plugins"));
        }
        other => panic!("expected a missing-plugin refusal, got {other:?}"),
    }
    for plugin in ["px4xplane", "PilotageAutoFlight", "PilotageCamera"] {
        let dir = root.join(format!("Resources/plugins/{plugin}/64"));
        std::fs::create_dir_all(&dir).expect("plugin dir");
        std::fs::write(dir.join("mac.xpl"), b"stub").expect("plugin stub");
    }
    let refusal = validate_xplane_install(&root, qtailsitter());
    match refusal {
        Err(XtaskError::MissingArtifact { what, hint, .. }) => {
            assert_eq!(what, "Pilotage X-Plane weather plugin");
            assert!(hint.contains("build-xplane-plugins"));
        }
        other => panic!("expected a missing-weather-plugin refusal, got {other:?}"),
    }
    let weather = root.join("Resources/plugins/PilotageWeather/64");
    std::fs::create_dir_all(&weather).expect("weather plugin dir");
    std::fs::write(weather.join("mac.xpl"), b"stub").expect("weather plugin stub");

    // With every plugin present, the missing aircraft is the next hint.
    let refusal = validate_xplane_install(&root, qtailsitter());
    match refusal {
        Err(XtaskError::MissingArtifact { what, path, .. }) => {
            assert_eq!(what, "X-Plane aircraft for the selected airframe");
            assert!(path.ends_with("QuadTailsitter.acf"), "got {path:?}");
        }
        other => panic!("expected a missing-aircraft refusal, got {other:?}"),
    }
}

#[test]
fn root_discovery_prefers_the_override_then_the_registry() {
    let explicit = PathBuf::from("/explicit/xplane");
    assert_eq!(
        xplane_root_from(Some(explicit.clone()), "ignored"),
        Some(explicit)
    );

    let root = scaffold("root-discovery");
    std::fs::create_dir_all(root.join("X-Plane.app")).expect("app dir");
    let registry = format!("/does/not/exist\n{}\n", root.display());
    assert_eq!(xplane_root_from(None, &registry), Some(root));
    assert_eq!(xplane_root_from(None, "/does/not/exist\n"), None);
}

#[test]
fn command_datagram_is_prologue_path_nul() {
    assert_eq!(
        command_datagram("px4xplane/toggleEnable"),
        [b"CMND\0".as_slice(), b"px4xplane/toggleEnable\0".as_slice()].concat()
    );
}

#[test]
fn config_rewrite_targets_only_the_config_name_line() {
    let root = scaffold("config-rewrite");
    let config_dir = root.join("Resources/plugins/px4xplane/64");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    let config = config_dir.join("config.ini");
    std::fs::write(&config, "; comment\nconfig_name = Alia250\nother = 1\n").expect("seed");
    set_active_config_name(&root, qtailsitter());
    let rewritten = std::fs::read_to_string(&config).expect("rewritten");
    assert_eq!(
        rewritten,
        "; comment\nconfig_name = QuadTailsitter\nother = 1\n"
    );
}
