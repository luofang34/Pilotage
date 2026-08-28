//! Plan and host-environment tests for the px4-xplane backend: profile
//! refusal, fail-closed airframe selection, install validation with
//! actionable hints, root discovery, and the CMND wire shape.

#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use super::super::xplane_simulator::{
    Airframe, airframe_for, classify_xplane_probe_status, command_datagram,
    plugin_install_required, record_plugin_install_stamp, run_plugin_build_blocking,
    run_xplane_prepare_sequence, set_active_config_name, validate_xplane_install,
    verify_weather_plugin_blocking, xplane_root_from, xplane_running_with_program_blocking,
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
fn stale_plugins_refuse_each_retry_without_a_restart() {
    assert!(!plugin_install_required(true, true).expect("current plugins"));
    assert!(plugin_install_required(false, false).expect("stopped simulator"));
    for _attempt in 0..2 {
        assert!(matches!(
            plugin_install_required(false, true),
            Err(XtaskError::SimulatorCapability { .. })
        ));
    }
}

#[test]
fn post_build_refusal_keeps_the_retry_stale() {
    use crate::session::preflight::stamp;

    let root = scaffold("post-build-refusal");
    let stamp_path = root.join("xplane-plugins.stamp");
    let current = "stopped simulator install contract\n";
    assert!(matches!(
        record_plugin_install_stamp(&stamp_path, Some(current), true),
        Err(XtaskError::SimulatorCapability { .. })
    ));
    assert!(
        !stamp_path.exists(),
        "a refused install must not look fresh"
    );

    let stored = stamp::read_stamp(&stamp_path);
    let plugins_current = stamp::artifact_is_fresh(true, stored.as_deref(), Some(current));
    assert!(!plugins_current);
    assert!(matches!(
        plugin_install_required(plugins_current, true),
        Err(XtaskError::SimulatorCapability { .. })
    ));
}

#[test]
fn xplane_process_probe_fails_closed_on_probe_errors() {
    let root = scaffold("process-probe");
    assert!(matches!(
        xplane_running_with_program_blocking(&root.join("missing-pgrep")),
        Err(XtaskError::Io { .. })
    ));
    assert!(matches!(
        xplane_running_with_program_blocking(std::path::Path::new("/usr/bin/true")),
        Ok(true)
    ));
    assert!(matches!(
        xplane_running_with_program_blocking(std::path::Path::new("/usr/bin/false")),
        Ok(false)
    ));
    let unexpected = std::process::Command::new("/bin/sh")
        .args(["-c", "exit 2"])
        .status()
        .expect("status 2");
    assert!(matches!(
        classify_xplane_probe_status(unexpected),
        Err(XtaskError::CommandFailed { .. })
    ));
}

#[test]
fn plugin_build_spawn_and_exit_failures_keep_their_types() {
    let root = scaffold("plugin-build-failures");
    let missing = root.join("missing-command");
    assert!(matches!(
        run_plugin_build_blocking(&root, &missing),
        Err(XtaskError::Io { .. })
    ));
    assert!(matches!(
        run_plugin_build_blocking(&root, std::path::Path::new("/usr/bin/false")),
        Err(XtaskError::CommandFailed { .. })
    ));
}

#[test]
fn weather_clear_proof_controls_controller_start() {
    let root = scaffold("weather-proof-failure");
    let scripts = root.join("scripts");
    std::fs::create_dir_all(&scripts).expect("scripts dir");
    std::fs::write(
        scripts.join("xplane_weather_clear.py"),
        "raise SystemExit(7)\n",
    )
    .expect("weather proof");
    assert!(matches!(
        verify_weather_plugin_blocking(&root),
        Err(XtaskError::SimulatorCapability { .. })
    ));
    std::fs::write(
        scripts.join("xplane_weather_clear.py"),
        "raise SystemExit(0)\n",
    )
    .expect("weather proof");
    assert!(verify_weather_plugin_blocking(&root).is_ok());
}

#[test]
fn xplane_prepare_sequence_proves_weather_before_connect_or_fc_start() {
    use std::cell::RefCell;

    let warm_trace = RefCell::new(Vec::new());
    run_xplane_prepare_sequence(
        true,
        || {
            warm_trace.borrow_mut().push("proof");
            Ok(())
        },
        || {
            warm_trace.borrow_mut().push("launch");
            Ok(())
        },
        || {
            warm_trace.borrow_mut().push("cold-proof");
            Ok(())
        },
        || warm_trace.borrow_mut().push("connect"),
    )
    .expect("warm prepare");
    assert_eq!(warm_trace.into_inner(), ["proof", "connect"]);

    let cold_trace = RefCell::new(Vec::new());
    run_xplane_prepare_sequence(
        false,
        || {
            cold_trace.borrow_mut().push("warm-proof");
            Ok(())
        },
        || {
            cold_trace.borrow_mut().push("launch");
            Ok(())
        },
        || {
            cold_trace.borrow_mut().push("proof");
            Ok(())
        },
        || cold_trace.borrow_mut().push("connect"),
    )
    .expect("cold prepare");
    assert_eq!(cold_trace.into_inner(), ["launch", "proof"]);

    let refusal_trace = RefCell::new(Vec::new());
    let refusal = run_xplane_prepare_sequence(
        true,
        || {
            refusal_trace.borrow_mut().push("proof");
            Err(XtaskError::SimulatorCapability {
                capability: "test weather proof",
                detail: "refused".to_owned(),
            })
        },
        || {
            refusal_trace.borrow_mut().push("launch");
            Ok(())
        },
        || {
            refusal_trace.borrow_mut().push("cold-proof");
            Ok(())
        },
        || refusal_trace.borrow_mut().push("connect"),
    );
    assert!(matches!(
        refusal,
        Err(XtaskError::SimulatorCapability { .. })
    ));
    assert_eq!(refusal_trace.into_inner(), ["proof"]);
}

#[test]
fn config_rewrite_targets_only_the_config_name_line() {
    let root = scaffold("config-rewrite");
    let config_dir = root.join("Resources/plugins/px4xplane/64");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    let config = config_dir.join("config.ini");
    std::fs::write(&config, "; comment\nconfig_name = Alia250\nother = 1\n").expect("seed");
    set_active_config_name(&root, qtailsitter(), false).expect("a stopped simulator is written");
    let rewritten = std::fs::read_to_string(&config).expect("rewritten");
    assert_eq!(
        rewritten,
        "; comment\nconfig_name = QuadTailsitter\nother = 1\n"
    );
}

/// The bridge answers a flight controller's connection and then drops it
/// when its configuration names an aircraft other than the one X-Plane has
/// loaded. Neither side says why: the controller retries until its
/// readiness deadline and the session fails with nothing pointing at the
/// aircraft. A launcher that starts X-Plane chooses both and they agree; a
/// launcher that finds it running chooses only the configuration.
#[test]
fn a_running_simulator_with_another_aircraft_is_refused_by_name() {
    use crate::backend::xplane_simulator::{airframe_for, loaded_aircraft, verify_loaded_aircraft};

    let airframe = airframe_for(Some("alia250")).expect("a known airframe");
    let other = airframe_for(Some("qtailsitter")).expect("a known airframe");

    let log = format!(
        "0:00:00.000 I/WIN: Showing subscreen Main Menu\n\
         0:00:25.977 I/ACF: Loading airplane number 0 with {}\n",
        airframe.acf_path,
    );
    assert_eq!(loaded_aircraft(&log).as_deref(), Some(airframe.acf_path));

    let root = std::env::temp_dir().join(format!("plt_xplane_acf_{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("fixture root");
    std::fs::write(root.join("Log.txt"), &log).expect("fixture log");

    verify_loaded_aircraft(&root, airframe).expect("the loaded aircraft is the selected one");

    let refusal = verify_loaded_aircraft(&root, other);
    let message = refusal
        .expect_err("another aircraft is refused")
        .to_string();
    assert!(
        message.contains(other.acf_path) && message.contains(airframe.acf_path),
        "the refusal names both aircraft: {message}",
    );
    assert!(
        message.contains("PILOTAGE_XPLANE_AIRFRAME"),
        "the refusal says how to resolve it: {message}",
    );

    // No log to read is not a mismatch: this check turns a silent failure
    // into a named one and never invents one.
    std::fs::remove_file(root.join("Log.txt")).expect("remove fixture log");
    verify_loaded_aircraft(&root, other).expect("an unreadable log states no aircraft");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_configuration_that_would_change_under_a_running_simulator_is_refused() {
    // The bridge reads its configuration when it loads. Rewriting the file
    // under a running simulator reaches nothing the run will use, and the
    // launcher then digests the file it wrote — putting a claim in the trial
    // document that the running bridge does not match. It is the same
    // situation as an aircraft already loaded, so it gets the same discipline:
    // check, refuse, and say what the operator has to do.
    let root = scaffold("config-under-running-sim");
    let config_dir = root.join("Resources/plugins/px4xplane/64");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    let config = config_dir.join("config.ini");
    std::fs::write(&config, "config_name = Alia250\n").expect("seed");

    let refused = set_active_config_name(&root, qtailsitter(), true);
    assert!(
        matches!(refused, Err(XtaskError::SimulatorCapability { .. })),
        "a configuration the running bridge cannot read was written anyway"
    );
    assert_eq!(
        std::fs::read_to_string(&config).expect("read back"),
        "config_name = Alia250\n",
        "the file was rewritten under a running simulator"
    );
}

#[test]
fn a_configuration_that_already_matches_is_accepted_under_a_running_simulator() {
    // Refusing whenever the simulator is up would fail the normal case: it is
    // already running with the configuration this session needs. The refusal
    // is about a change that cannot take effect, not about the simulator.
    let root = scaffold("config-running-already-right");
    let config_dir = root.join("Resources/plugins/px4xplane/64");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    let config = config_dir.join("config.ini");
    let settled = format!("config_name = {}\n", qtailsitter().config_name);
    std::fs::write(&config, &settled).expect("seed");

    set_active_config_name(&root, qtailsitter(), true)
        .expect("a configuration that already matches is not a change");
    assert_eq!(
        std::fs::read_to_string(&config).expect("read back"),
        settled
    );
}
