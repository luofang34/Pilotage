//! Plan and host-environment tests for the px4-gz backend: profile
//! refusal, gimbal-capability wiring, actionable missing-artifact hints,
//! and the shared flight-deck world invariants.

#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use super::vehicle::Px4Model;
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
        lan: false,
        web_root: std::env::temp_dir(),
        instance: 0,
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
    let backend = Px4Gz::default();
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
    let backend = Px4Gz::default();
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
    let backend = Px4Gz::default();
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

    let refusal = plan_with_px4_dir(&ctx, &root.join("absent"), Px4Model::X500Gimbal);
    assert!(matches!(
        refusal,
        Err(XtaskError::MissingArtifact {
            what: "PX4-Autopilot checkout",
            ..
        })
    ));

    let refusal = plan_with_px4_dir(&ctx, &px4, Px4Model::X500Gimbal);
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

/// A scaffold whose every artifact exists, so planning succeeds.
fn complete_scaffold(tag: &str) -> (PathBuf, PathBuf) {
    let (root, px4) = scaffold(tag);
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
    (root, px4)
}

#[test]
fn a_complete_checkout_plans_gz_then_px4_standalone() {
    let (root, px4) = complete_scaffold("complete");
    let ctx = context(root.clone());
    let stages = plan_with_px4_dir(&ctx, &px4, Px4Model::X500Gimbal).expect("plan");
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

fn value<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
    env.iter()
        .find(|(k, _)| k == key)
        .map(|(_, value)| value.as_str())
}

/// Instance 0 is the single-session launch exactly as it always was: no
/// `-i`, the build tree's own rootfs, the default gz partition, and the
/// host on PX4's stock ports as system 1.
#[test]
fn instance_zero_plans_the_single_session_launch() {
    let (root, px4) = complete_scaffold("instance-zero");
    let ctx = context(root.clone());
    let stages = plan_with_px4_dir(&ctx, &px4, Px4Model::X500Gimbal).expect("plan");
    let fc = &stages[1].spec;
    assert!(!fc.args.contains(&"-i".to_owned()), "{:?}", fc.args);
    assert_eq!(
        fc.cwd.as_deref(),
        Some(px4.join("build/px4_sitl_default/rootfs").as_path())
    );
    for env in [&stages[0].spec.env, &fc.env] {
        assert_eq!(value(env, "GZ_PARTITION"), None);
    }
    let host = Px4Gz::default().host_env(&ctx);
    assert_eq!(value(&host, "PILOTAGE_PX4_ADDR"), Some("127.0.0.1:14550"));
    assert_eq!(
        value(&host, "PILOTAGE_PX4_GCS_ADDR"),
        Some("127.0.0.1:18570")
    );
    assert_eq!(
        value(&host, "PILOTAGE_PX4_FC_ADDR"),
        Some("127.0.0.1:14580")
    );
    assert_eq!(value(&host, "PILOTAGE_PX4_SYSTEM_ID"), Some("1"));
    assert_eq!(value(&host, "GZ_PARTITION"), None);
    std::fs::remove_dir_all(&root).ok();
}

/// Another instance moves EVERY process to its own copy: PX4 shifts its
/// ports with `-i`, the host follows those ports and system id, and gz,
/// its readiness probe, PX4's bridge, and the host's camera sidecar all
/// join one partition. A process left on the default partition would
/// talk to another session's world.
#[test]
fn another_instance_shifts_every_port_and_shares_one_partition() {
    let (root, px4) = complete_scaffold("instance-five");
    let mut ctx = context(root.clone());
    ctx.instance = 5;
    let stages = plan_with_px4_dir(&ctx, &px4, Px4Model::X500Gimbal).expect("plan");
    let fc = &stages[1].spec;
    let at = fc
        .args
        .iter()
        .position(|arg| arg == "-i")
        .expect("-i passed");
    assert_eq!(fc.args.get(at + 1).map(String::as_str), Some("5"));
    assert_eq!(
        fc.cwd.as_deref(),
        Some(px4.join("build/px4_sitl_default/rootfs/5").as_path()),
        "each instance keeps its own parameters and dataman state"
    );

    let host = Px4Gz::default().host_env(&ctx);
    assert_eq!(value(&host, "PILOTAGE_PX4_ADDR"), Some("127.0.0.1:14555"));
    assert_eq!(
        value(&host, "PILOTAGE_PX4_GCS_ADDR"),
        Some("127.0.0.1:18575")
    );
    assert_eq!(
        value(&host, "PILOTAGE_PX4_FC_ADDR"),
        Some("127.0.0.1:14585")
    );
    assert_eq!(value(&host, "PILOTAGE_PX4_SYSTEM_ID"), Some("6"));

    let partition = value(&host, "GZ_PARTITION").expect("the host's sidecar is partitioned");
    let crate::readiness::Readiness::CommandOutput { env: probe, .. } = &stages[0].readiness else {
        panic!("gz readiness is a probe command");
    };
    for (who, env) in [
        ("gz", &stages[0].spec.env),
        ("probe", probe),
        ("px4", &fc.env),
    ] {
        assert_eq!(value(env, "GZ_PARTITION"), Some(partition), "{who}");
    }
    std::fs::remove_dir_all(&root).ok();
}

/// The guard refuses only a true conflict: two instances claim nothing in
/// common, and the same instance claims exactly the same ports.
#[test]
fn instances_claim_disjoint_ports_and_only_instance_zero_needs_the_default_partition() {
    use crate::conflicts::Claim;

    let claims = |instance| {
        let mut ctx = context(PathBuf::from("unused"));
        ctx.instance = instance;
        Px4Gz::default().claims(&ctx)
    };
    let zero = claims(0);
    let five = claims(5);
    assert!(zero.contains(&Claim::DefaultGzPartition));
    assert!(!five.contains(&Claim::DefaultGzPartition));
    let ports = |claims: &[Claim]| -> Vec<u16> {
        claims
            .iter()
            .filter_map(|claim| match claim {
                Claim::Udp { port, .. } => Some(*port),
                _ => None,
            })
            .collect()
    };
    assert_eq!(ports(&zero), vec![18_570, 14_580, 14_550]);
    assert_eq!(ports(&five), vec![18_575, 14_585, 14_555]);
    assert_eq!(claims(5), five, "the same instance claims the same ports");
}

#[test]
fn an_instance_past_the_port_plan_is_refused() {
    let mut ctx = context(PathBuf::from("unused"));
    ctx.instance = 30;
    assert!(matches!(
        Px4Gz::default().plan(&ctx),
        Err(XtaskError::Usage { .. })
    ));
}

/// Each model's world includes the vehicle under the exact name PX4
/// attaches to, from the gz model the airframe's `PX4_SIM_MODEL` names,
/// with the camera rig welded to that vehicle. A world that disagrees
/// boots cleanly and then flies nothing.
#[test]
fn every_model_world_carries_the_vehicle_its_airframe_attaches_to() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root");
    for model in Px4Model::ALL {
        let world = std::fs::read_to_string(repo_root.join(model.world()))
            .unwrap_or_else(|error| panic!("{}: {error}", model.world()));
        let gz_model = model
            .sim_model()
            .strip_prefix("gz_")
            .expect("PX4 gz models are named gz_<model>");
        assert!(
            world.contains(&format!("<uri>model://{gz_model}</uri>")),
            "{}: includes {gz_model}",
            model.name()
        );
        assert!(
            world.contains(&format!("<name>{}</name>", model.model_name())),
            "{}: names the vehicle {}",
            model.name(),
            model.model_name()
        );
        assert!(
            world.contains(&format!(
                "<parent>{}::base_link</parent>",
                model.model_name()
            )),
            "{}: the camera rig rides on the vehicle",
            model.name()
        );
        assert!(
            world.contains("<world name=\"default\">"),
            "{}",
            model.name()
        );
    }
}

#[test]
fn a_model_is_selected_by_name_and_an_unknown_one_is_refused() {
    use crate::backend::backend_for;

    for spec in ["px4-gz", "px4", "px4-gz:x500_gimbal", "px4-gz:rc_cessna"] {
        backend_for(spec).unwrap_or_else(|error| panic!("{spec}: {error}"));
    }
    for spec in ["px4-gz:x500", "px4-gz:", "aviate-gz:x500"] {
        assert!(
            matches!(backend_for(spec), Err(XtaskError::UnknownModel { .. })),
            "{spec} must be refused"
        );
    }
    assert!(matches!(
        backend_for("ardupilot-gz:rc_cessna"),
        Err(XtaskError::UnknownBackend { .. })
    ));
}

/// The host learns how the vehicle flies, and only a vehicle with a
/// gimbal advertises one.
#[test]
fn the_host_learns_the_airframe_and_the_gimbal_of_the_selected_model() {
    let ctx = context(PathBuf::from("unused"));
    let host = |model: &str| {
        let mut backend = Px4Gz::default();
        backend.select_model(model).expect("known model");
        backend.host_env(&ctx)
    };
    let quad = host("x500_gimbal");
    assert_eq!(value(&quad, "PILOTAGE_PX4_AIRFRAME"), Some("multicopter"));
    assert_eq!(value(&quad, "PILOTAGE_PX4_GIMBAL"), Some("1"));
    let plane = host("rc_cessna");
    assert_eq!(value(&plane, "PILOTAGE_PX4_AIRFRAME"), Some("fixed_wing"));
    assert_eq!(value(&plane, "PILOTAGE_PX4_GIMBAL"), None);
}

#[test]
fn the_cessna_plans_its_own_world_and_airframe() {
    let (root, px4) = complete_scaffold("cessna");
    let world = root.join(Px4Model::RcCessna.world());
    std::fs::write(&world, b"x").expect("world");
    let ctx = context(root.clone());
    let stages = plan_with_px4_dir(&ctx, &px4, Px4Model::RcCessna).expect("plan");
    assert!(stages[0].spec.args.contains(&world.display().to_string()));
    let fc = &stages[1].spec.env;
    assert_eq!(value(fc, "PX4_SYS_AUTOSTART"), Some("4003"));
    assert_eq!(value(fc, "PX4_SIM_MODEL"), Some("gz_rc_cessna"));
    assert_eq!(value(fc, "PX4_GZ_MODEL_NAME"), Some("rc_cessna_0"));
    std::fs::remove_dir_all(&root).ok();
}
