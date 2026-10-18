//! Plan and environment tests for the aviate-gz backend.

#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use super::plan_with_aviate_dir;
use crate::backend::SessionContext;
use crate::cli::Profile;
use crate::error::XtaskError;

fn context(repo_root: PathBuf) -> SessionContext {
    let log_dir = repo_root.join("target/xtask-sim");
    SessionContext {
        repo_root,
        host_port: 4433,
        viewer_port: 8080,
        profile: Profile::Simulation,
        log_dir,
        lan: false,
        web_root: std::env::temp_dir(),
        instance: 0,
    }
}

fn scaffold(tag: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("plt_xtask_{tag}_{}", std::process::id()));
    let repo = base.join("repo");
    let aviate = base.join("Aviate");
    std::fs::create_dir_all(repo.join("sim/worlds")).expect("repo dirs");
    std::fs::create_dir_all(aviate.join("aviate-hal/xil/backends/gz/plugin/build"))
        .expect("plugin dir");
    std::fs::create_dir_all(aviate.join("target/debug")).expect("fc dir");
    (repo, aviate)
}

#[test]
fn missing_artifacts_fail_with_actionable_hints() {
    let (repo, aviate) = scaffold("hints");
    let ctx = context(repo.clone());

    let refusal = plan_with_aviate_dir(&ctx, &aviate.join("nowhere"));
    assert!(matches!(
        refusal,
        Err(XtaskError::MissingArtifact {
            what: "Aviate checkout",
            ..
        })
    ));

    let refusal = plan_with_aviate_dir(&ctx, &aviate);
    assert!(matches!(
        refusal,
        Err(XtaskError::MissingArtifact {
            what: "Aviate gz plugin",
            ..
        })
    ));

    std::fs::write(
        aviate.join("aviate-hal/xil/backends/gz/plugin/build/libAviateGzPlugin.dylib"),
        b"",
    )
    .expect("plugin");
    let refusal = plan_with_aviate_dir(&ctx, &aviate);
    assert!(matches!(
        refusal,
        Err(XtaskError::MissingArtifact {
            what: "Aviate SITL FC binary",
            ..
        })
    ));

    std::fs::write(aviate.join("target/debug/sitl-gazebo-x500"), b"").expect("fc");
    let refusal = plan_with_aviate_dir(&ctx, &aviate);
    assert!(matches!(
        refusal,
        Err(XtaskError::MissingArtifact {
            what: "flight-deck world",
            ..
        })
    ));
    std::fs::remove_dir_all(repo.parent().expect("base")).ok();
}

#[test]
fn a_complete_checkout_plans_gz_then_fc_with_the_runbook_environment() {
    let (repo, aviate) = scaffold("plan");
    std::fs::write(
        aviate.join("aviate-hal/xil/backends/gz/plugin/build/libAviateGzPlugin.dylib"),
        b"",
    )
    .expect("plugin");
    std::fs::write(aviate.join("target/debug/sitl-gazebo-x500"), b"").expect("fc");
    std::fs::write(repo.join("sim/worlds/x500_flightdeck.sdf"), b"<sdf/>").expect("world");

    let ctx = context(repo.clone());
    let stages = plan_with_aviate_dir(&ctx, &aviate).expect("plans");
    assert_eq!(stages.len(), 2);
    assert_eq!(stages[0].spec.name, "gazebo");
    assert_eq!(stages[1].spec.name, "flight-controller");

    let env = |stage: usize, key: &str| -> String {
        stages[stage]
            .spec
            .env
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("{key} missing"))
    };
    assert!(env(0, "GZ_SIM_SYSTEM_PLUGIN_PATH").contains("plugin/build"));
    assert!(env(0, "GZ_SIM_RESOURCE_PATH").contains("PX4-gazebo-models"));
    assert!(env(0, "GZ_SIM_RESOURCE_PATH").contains("sim/worlds"));
    assert_eq!(env(0, "GZ_IP"), "127.0.0.1");
    assert!(
        stages[0].spec.remove_env.contains(&"DISPLAY"),
        "gz must run headless without an X display"
    );
    assert_eq!(env(1, "GZ_IP"), "127.0.0.1");
    std::fs::remove_dir_all(repo.parent().expect("base")).ok();
}

/// A sensor's topic is scoped by the world it is in. The launcher
/// chooses the world and the host builds the topic, so they agree
/// through this name and nothing else. A typo in either place
/// subscribes successfully to a topic nobody publishes: gz-transport
/// reports no error, the bridge reports no fix, and the map states no
/// position with nothing anywhere to say why.
#[test]
fn the_launcher_names_the_world_the_sensor_topic_is_scoped_by() {
    use crate::backend::SimBackend;

    let ctx = context(PathBuf::from("unused-for-host-environment"));
    let env = super::AviateGz.host_env(&ctx);
    let world = env
        .iter()
        .find(|(key, _)| key == "PILOTAGE_GZ_WORLD")
        .map(|(_, value)| value.clone())
        .expect("the launcher tells the host which world it started");
    assert_eq!(world, super::WORLD_NAME);
    assert!(
        !world.is_empty(),
        "an empty world name builds no topic and the host reads no position"
    );
}

/// The FC streams where the host listens, on every instance; instance
/// 0 keeps the shared default and any other instance moves clear of
/// every PX4 port.
#[test]
fn both_telemetry_ends_move_together_and_clear_of_px4() {
    use crate::backend::SimBackend;
    use crate::conflicts::Claim;

    for instance in [0_u16, 1, 5, 29] {
        let (repo, aviate) = scaffold(&format!("telemetry-{instance}"));
        std::fs::write(
            aviate.join("aviate-hal/xil/backends/gz/plugin/build/libAviateGzPlugin.dylib"),
            b"",
        )
        .expect("plugin");
        std::fs::write(aviate.join("target/debug/sitl-gazebo-x500"), b"").expect("fc");
        std::fs::write(repo.join("sim/worlds/x500_flightdeck.sdf"), b"<sdf/>").expect("world");
        let mut ctx = context(repo.clone());
        ctx.instance = instance;
        let stages = plan_with_aviate_dir(&ctx, &aviate).expect("plans");
        let value = |env: &[(String, String)], key: &str| {
            env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
        };
        let fc = value(&stages[1].spec.env, "AVIATE_TELEMETRY_ENDPOINT").expect("fc end");
        let host =
            value(&super::AviateGz.host_env(&ctx), "PILOTAGE_AVIATE_ADDR").expect("host end");
        assert_eq!(fc, host, "instance {instance}");
        let port: u16 = host
            .rsplit(':')
            .next()
            .and_then(|p| p.parse().ok())
            .expect("port");
        if instance == 0 {
            assert_eq!(port, 14_550);
        } else {
            let px4 = crate::backend::px4_gz::ports::Px4Ports::for_instance(instance);
            for taken in [px4.gcs_local, px4.offboard_local, px4.host_telemetry] {
                assert_ne!(port, taken, "instance {instance}");
            }
            assert!(
                !(14_550..=14_609).contains(&port),
                "instance {instance}: {port}"
            );
        }
        assert!(
            super::AviateGz
                .claims(&ctx)
                .iter()
                .any(|claim| matches!(claim, Claim::Udp { port: claimed, .. } if *claimed == port))
        );
        std::fs::remove_dir_all(repo.parent().expect("base")).ok();
    }
}
