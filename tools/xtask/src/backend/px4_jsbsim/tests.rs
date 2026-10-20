//! Plan, placement, and claim tests for the px4-jsbsim backend.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use super::{JsbsimModel, Px4Jsbsim, plan_with_dirs, simulator_port};
use crate::backend::{Placement, SessionContext, SimBackend, backend_for};
use crate::cli::Profile;
use crate::conflicts::Claim;
use crate::error::XtaskError;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plt_xt_px4_jsbsim_{tag}_{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A PX4 checkout with a SITL build, and a built JSBSim.
fn scaffold(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = scratch(tag);
    let px4 = root.join("PX4-Autopilot");
    std::fs::create_dir_all(px4.join("build/px4_sitl_default/bin")).unwrap();
    std::fs::write(px4.join("build/px4_sitl_default/bin/px4"), "").unwrap();
    let jsbsim = root.join("jsbsim");
    for dir in [
        "aircraft",
        "engine",
        "install/include/JSBSim",
        "install/lib/pkgconfig",
    ] {
        std::fs::create_dir_all(jsbsim.join(dir)).unwrap();
    }
    std::fs::write(jsbsim.join("install/include/JSBSim/FGFDMExec.h"), "").unwrap();
    std::fs::write(jsbsim.join("install/lib/libJSBSim.a"), "").unwrap();
    std::fs::write(
        jsbsim.join("install/lib/pkgconfig/JSBSim.pc"),
        "Version: 1.3.3\n",
    )
    .unwrap();
    (root, px4, jsbsim)
}

fn context(root: &Path, instance: u16) -> SessionContext {
    let mut ctx = SessionContext::for_test(root.to_path_buf());
    ctx.log_dir = root.join("logs");
    std::fs::create_dir_all(&ctx.log_dir).unwrap();
    ctx.instance = instance;
    ctx
}

fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let at = args.iter().position(|arg| arg == flag)?;
    args.get(at + 1).map(String::as_str)
}

fn env<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
    env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

#[test]
fn the_backend_is_selected_by_name_with_its_models() {
    assert_eq!(backend_for("px4-jsbsim").unwrap().name(), "px4-jsbsim");
    assert_eq!(
        backend_for("px4-jsbsim:plane").unwrap().ground(),
        crate::site::Ground::Runway
    );
    assert_eq!(
        backend_for("px4-jsbsim:quad").unwrap().ground(),
        crate::site::Ground::Field
    );
    match backend_for("px4-jsbsim:x500_gimbal") {
        Err(XtaskError::UnknownModel { known, .. }) => assert_eq!(known, ["quad", "plane"]),
        Err(other) => panic!("expected an unknown model, got {other}"),
        Ok(_) => panic!("an unknown model was accepted"),
    }
}

#[test]
fn the_quad_plans_the_sidecar_then_px4_on_the_instance_port() {
    let (root, px4, jsbsim) = scaffold("quad");
    let stages = plan_with_dirs(&context(&root, 21), &px4, &jsbsim, JsbsimModel::Quad).unwrap();
    assert_eq!(stages.len(), 2);
    let sidecar = &stages[0].spec;
    assert_eq!(sidecar.name, "jsbsim");
    assert!(
        sidecar
            .program
            .ends_with("target/release/pilotage-jsbsim-sidecar")
    );
    assert_eq!(arg(&sidecar.args, "--port"), Some("4581"));
    assert_eq!(arg(&sidecar.args, "--aircraft"), Some("x500"));
    assert_eq!(arg(&sidecar.args, "--truth-addr"), Some("127.0.0.1:19581"));
    assert_eq!(arg(&sidecar.args, "--aircraft-dir"), None);
    assert_eq!(
        arg(&sidecar.args, "--jsbsim-root"),
        Some(jsbsim.to_str().unwrap())
    );
    let px4_stage = &stages[1].spec;
    assert_eq!(px4_stage.name, "flight-controller");
    assert_eq!(arg(&px4_stage.args, "-i"), Some("21"));
    assert_eq!(env(&px4_stage.env, "PX4_SYS_AUTOSTART"), Some("3010"));
    assert_eq!(env(&px4_stage.env, "PX4_PARAM_CA_ROTOR0_PX"), Some("0.174"));
    assert_eq!(env(&px4_stage.env, "PX4_PARAM_SDLOG_MODE"), Some("-1"));
    assert!(px4_stage.remove_env.contains(&"PX4_SIMULATOR"));
    // PX4 drives the gimbal on outputs 5-7 with the travel the sidecar's
    // gimbal model maps them back with.
    assert_eq!(env(&px4_stage.env, "PX4_PARAM_MNT_MODE_OUT"), Some("0"));
    assert_eq!(env(&px4_stage.env, "PX4_PARAM_MNT_DO_STAB"), Some("1"));
    assert_eq!(env(&px4_stage.env, "PX4_PARAM_PWM_MAIN_FUNC6"), Some("421"));
    let travel = [
        env(&px4_stage.env, "PX4_PARAM_MNT_RANGE_ROLL").unwrap(),
        env(&px4_stage.env, "PX4_PARAM_MNT_MAX_PITCH").unwrap(),
        env(&px4_stage.env, "PX4_PARAM_MNT_MIN_PITCH").unwrap(),
        env(&px4_stage.env, "PX4_PARAM_MNT_RANGE_YAW").unwrap(),
    ];
    assert_eq!(
        arg(&sidecar.args, "--gimbal"),
        Some(travel.join(",").as_str())
    );
    assert!(px4_stage.cwd.as_ref().unwrap().ends_with("rootfs/21"));
}

#[test]
fn the_plane_flies_the_rascal_with_airframe_1033() {
    let (root, px4, jsbsim) = scaffold("plane");
    let stages = plan_with_dirs(&context(&root, 0), &px4, &jsbsim, JsbsimModel::Plane).unwrap();
    let sidecar = &stages[0].spec;
    assert_eq!(arg(&sidecar.args, "--port"), Some("4560"));
    assert_eq!(arg(&sidecar.args, "--aircraft"), Some("rascal"));
    assert_eq!(arg(&sidecar.args, "--aircraft-dir"), None);
    // A plain session starts on the N51 runway 04 threshold.
    assert_eq!(arg(&sidecar.args, "--latitude-deg"), Some("40.5771861"));
    assert_eq!(arg(&sidecar.args, "--heading-deg"), Some("29.50"));
    assert_eq!(env(&stages[1].spec.env, "PX4_SYS_AUTOSTART"), Some("1033"));
    assert_eq!(arg(&sidecar.args, "--gimbal"), None);
    let params: Vec<&str> = stages[1]
        .spec
        .env
        .iter()
        .filter_map(|(k, _)| k.strip_prefix("PX4_PARAM_"))
        .collect();
    assert_eq!(params, ["SDLOG_MODE"]);
}

#[test]
fn missing_artifacts_name_their_fix() {
    let (root, px4, jsbsim) = scaffold("missing");
    std::fs::remove_file(jsbsim.join("install/lib/libJSBSim.a")).unwrap();
    match plan_with_dirs(&context(&root, 0), &px4, &jsbsim, JsbsimModel::Quad) {
        Err(XtaskError::MissingArtifact { hint, .. }) => assert!(hint.contains("build-jsbsim")),
        other => panic!("expected a missing JSBSim, got {other:?}"),
    }
    let (root, px4, jsbsim) = scaffold("no_px4");
    std::fs::remove_file(px4.join("build/px4_sitl_default/bin/px4")).unwrap();
    assert!(matches!(
        plan_with_dirs(&context(&root, 0), &px4, &jsbsim, JsbsimModel::Plane),
        Err(XtaskError::MissingArtifact {
            what: "PX4 SITL binary",
            ..
        })
    ));
    let mut physical = context(&root, 0);
    physical.profile = Profile::Physical;
    assert!(matches!(
        plan_with_dirs(&physical, &px4, &jsbsim, JsbsimModel::Quad),
        Err(XtaskError::Usage { .. })
    ));
    assert!(matches!(
        plan_with_dirs(&context(&root, 30), &px4, &jsbsim, JsbsimModel::Quad),
        Err(XtaskError::Usage { .. })
    ));
}

#[test]
fn a_fleet_places_the_vehicle_through_the_sidecar() {
    let (root, px4, jsbsim) = scaffold("spawn");
    let ctx = context(&root, 3);
    let mut stages = plan_with_dirs(&ctx, &px4, &jsbsim, JsbsimModel::Quad).unwrap();
    let point = Placement {
        latitude_deg: 40.578,
        longitude_deg: -74.737,
        elevation_m: 61.25,
        heading_deg: 29.5,
    };
    Px4Jsbsim::default()
        .spawn(&ctx, &point, &mut stages)
        .unwrap();
    let args = &stages[0].spec.args;
    assert_eq!(arg(args, "--latitude-deg"), Some("40.5780000"));
    assert_eq!(arg(args, "--longitude-deg"), Some("-74.7370000"));
    assert_eq!(arg(args, "--site-elevation-m"), Some("61.25"));
}

#[test]
fn an_instance_claims_its_simulator_port_and_px4s_links() {
    let ctx = context(&scratch("claims"), 22);
    let claims = Px4Jsbsim::default().claims(&ctx);
    assert!(
        claims
            .iter()
            .any(|claim| matches!(claim, Claim::Tcp { port: 4582, .. }))
    );
    for port in [18_592, 14_602, 14_572] {
        assert!(
            claims
                .iter()
                .any(|claim| matches!(claim, Claim::Udp { port: p, .. } if *p == port)),
            "{port} unclaimed"
        );
    }
    assert!(
        claims
            .iter()
            .any(|claim| matches!(claim, Claim::Udp { port: 19_582, .. }))
    );
    assert_eq!(simulator_port(22), 4582);
}

#[test]
fn the_host_flies_the_selected_airframe_on_the_instance_links() {
    let ctx = context(&scratch("host"), 20);
    let mut backend = Px4Jsbsim::default();
    backend.select_model("plane").unwrap();
    let host = backend.host_env(&ctx);
    assert_eq!(env(&host, "PILOTAGE_PX4_AIRFRAME"), Some("fixed_wing"));
    assert_eq!(env(&host, "PILOTAGE_PX4_ADDR"), Some("127.0.0.1:14570"));
    assert_eq!(env(&host, "PILOTAGE_PX4_SYSTEM_ID"), Some("21"));
    assert!(env(&host, "GZ_IP").is_none());
    // The host takes truth from the sidecar, on the port the sidecar sends to.
    assert_eq!(
        env(&host, "PILOTAGE_SIM_TRUTH_ADDR"),
        Some("127.0.0.1:19580")
    );
    assert_eq!(env(&host, "PILOTAGE_PX4_GIMBAL"), None);
    let quad = Px4Jsbsim::default().host_env(&ctx);
    assert_eq!(env(&quad, "PILOTAGE_PX4_GIMBAL"), Some("1"));
}
