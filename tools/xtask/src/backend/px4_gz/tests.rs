//! Plan and host-environment tests for the px4-gz backend: profile
//! refusal, gimbal-capability wiring, actionable missing-artifact hints,
//! and the shared flight-deck world invariants.

#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use super::{Px4Gz, plan_with_px4_dir};
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
    }
}

fn scaffold(tag: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("px4-gz-plan-{tag}-{}", std::process::id()));
    let px4 = root.join("PX4-Autopilot");
    std::fs::create_dir_all(&px4).expect("scaffold");
    (root, px4)
}

#[test]
fn plan_refuses_physical_and_oracle_only_profiles() {
    let backend = Px4Gz;
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
fn host_environment_declares_the_px4_simulation_profile() {
    let backend = Px4Gz;
    let ctx = context(PathBuf::from("unused-for-host-environment"));
    assert!(
        backend
            .host_env(&ctx)
            .iter()
            .any(|(key, value)| { key == "PILOTAGE_PX4_PROFILE" && value == "simulation" })
    );
}

#[test]
fn host_environment_enables_the_gimbal_capability() {
    // The gz_x500_gimbal airframe carries a gimbal; the host must
    // advertise the scope, and no other FC backend sets this flag.
    let backend = Px4Gz;
    let ctx = context(PathBuf::from("unused-for-host-environment"));
    assert!(
        backend
            .host_env(&ctx)
            .iter()
            .any(|(key, value)| key == "PILOTAGE_PX4_GIMBAL" && value == "1")
    );
}

#[test]
fn missing_artifacts_fail_with_actionable_hints() {
    let (root, px4) = scaffold("missing");
    let ctx = context(root.clone());

    let refusal = plan_with_px4_dir(&ctx, &root.join("absent"));
    assert!(matches!(
        refusal,
        Err(XtaskError::MissingArtifact {
            what: "PX4-Autopilot checkout",
            ..
        })
    ));

    let refusal = plan_with_px4_dir(&ctx, &px4);
    assert!(matches!(
        refusal,
        Err(XtaskError::MissingArtifact {
            what: "PX4 SITL binary",
            ..
        })
    ));
    std::fs::remove_dir_all(&root).ok();
}

// The two FC families keep separate worlds (their physics steps,
// vehicle models, and FC glue genuinely differ), but the flight
// deck must LOOK the same from the cameras: one green field, one
// sun, one rig. This pins the shared appearance so the two files
// cannot drift apart silently.
#[test]
fn both_flight_deck_worlds_share_the_same_look() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root");
    let aviate = std::fs::read_to_string(repo_root.join("sim/worlds/x500_flightdeck.sdf"))
        .expect("aviate world");
    let px4 = std::fs::read_to_string(repo_root.join("sim/worlds/px4_flightdeck.sdf"))
        .expect("px4 world");
    for invariant in [
        "<uri>model://flightdeck_scenery</uri>",
        "<direction>-0.5 0.1 -0.9</direction>",
        "<magnetic_field>",
        "<model name=\"x500_camera_rig\">",
        "<topic>camera</topic>",
        "<topic>chase_camera</topic>",
    ] {
        assert!(aviate.contains(invariant), "aviate world lost {invariant}");
        assert!(px4.contains(invariant), "px4 world lost {invariant}");
    }
    // Neither world may carry its own ground: the field lives in
    // the ONE shared scenery model (green, 500 m) so future props
    // appear for every FC family at once.
    assert!(!aviate.contains("ground_plane") && !px4.contains("ground_plane"));
    let scenery =
        std::fs::read_to_string(repo_root.join("sim/models/flightdeck_scenery/model.sdf"))
            .expect("scenery model");
    assert!(scenery.contains("<ambient>0.3 0.5 0.3 1</ambient>"));
    assert!(scenery.contains("<size>500 500</size>"));
    // The default sky is part of the look: neither world may
    // override the scene with a gray background.
    assert!(!aviate.contains("<scene>") && !px4.contains("<scene>"));
}

#[test]
fn a_complete_checkout_plans_gz_then_px4_standalone() {
    let (root, px4) = scaffold("complete");
    for file in [
        "build/px4_sitl_default/bin/px4",
        "Tools/simulation/gz/server.config",
    ] {
        let path = px4.join(file);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(&path, b"x").expect("file");
    }
    let world = root.join("sim/worlds/px4_flightdeck.sdf");
    std::fs::create_dir_all(world.parent().expect("parent")).expect("dirs");
    std::fs::write(&world, b"x").expect("world");
    let ctx = context(root.clone());
    let stages = plan_with_px4_dir(&ctx, &px4).expect("plan");
    assert_eq!(stages.len(), 2);
    assert_eq!(stages[0].spec.name, "gazebo");
    assert_eq!(stages[1].spec.name, "flight-controller");
    let fc_env = &stages[1].spec.env;
    assert!(
        fc_env
            .iter()
            .any(|(k, v)| k == "PX4_GZ_STANDALONE" && v == "1"),
        "px4 must attach to the xtask-owned gz server, not spawn its own"
    );
    assert!(
        fc_env
            .iter()
            .any(|(k, v)| k == "PX4_GZ_MODEL_NAME" && v == "x500_0"),
        "px4 must attach to the world's rig-bearing model, not spawn one"
    );
    assert!(
        stages[0]
            .spec
            .env
            .iter()
            .any(|(k, _)| k == "GZ_SIM_SERVER_CONFIG_PATH"),
        "gz must load PX4's sensor systems via the server config"
    );
    assert!(
        fc_env
            .iter()
            .any(|(k, v)| k == "PX4_SYS_AUTOSTART" && v == "4019")
            && fc_env
                .iter()
                .any(|(k, v)| k == "PX4_SIM_MODEL" && v == "gz_x500_gimbal"),
        "the FC must boot the gimbal airframe (GIM-04): 4019/gz_x500_gimbal"
    );
    std::fs::remove_dir_all(&root).ok();
}

// The airframe env above and the statically included world model
// must agree: PX4's GZGimbal bridge publishes joint commands under
// `/model/<PX4_GZ_MODEL_NAME>/…`, so a world still carrying the
// plain x500 would boot cleanly and then ignore every gimbal
// demand silently.
#[test]
fn the_px4_world_carries_the_gimbal_model_the_airframe_expects() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root");
    let world = std::fs::read_to_string(repo_root.join("sim/worlds/px4_flightdeck.sdf"))
        .expect("px4 world");
    assert!(
        world.contains("<uri>model://x500_gimbal</uri>"),
        "the px4 world must include the gimbal-bearing vehicle"
    );
    assert!(
        world.contains("<name>x500_0</name>"),
        "the included vehicle must keep the name PX4_GZ_MODEL_NAME attaches to"
    );
}
