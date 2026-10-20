//! Plan, placement, host and claim tests for the aviate-jsbsim backend.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use super::{AviateJsbsim, command_port, plan_with_dirs};
use crate::backend::{Placement, SessionContext, SimBackend, backend_for};
use crate::conflicts::Claim;
use crate::error::XtaskError;

fn scratch(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("plt_xt_aviate_jsbsim_{tag}_{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An Aviate checkout with the JSBSim FC built, and a built JSBSim.
fn scaffold(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = scratch(tag);
    let aviate = root.join("Aviate");
    std::fs::create_dir_all(aviate.join("target/debug")).unwrap();
    std::fs::write(aviate.join("target/debug/sitl-jsbsim-x500"), "").unwrap();
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
    (root, aviate, jsbsim)
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
fn the_backend_is_selected_by_name_and_flies_a_quad_on_a_field() {
    let backend = backend_for("aviate-jsbsim").unwrap();
    assert_eq!(backend.name(), "aviate-jsbsim");
    assert_eq!(backend.host_adapter(), "aviate");
    assert_eq!(backend.ground(), crate::site::Ground::Field);
    assert!(backend.control_feel_profile().is_some());
}

#[test]
fn the_fc_dials_the_sidecar_of_its_own_instance() {
    let (root, aviate, jsbsim) = scaffold("plan");
    let stages = plan_with_dirs(&context(&root, 20), &aviate, &jsbsim).unwrap();
    assert_eq!(stages.len(), 2);
    let sidecar = &stages[0].spec;
    assert_eq!(sidecar.name, "jsbsim");
    assert_eq!(arg(&sidecar.args, "--port"), Some("4580"));
    assert_eq!(arg(&sidecar.args, "--aircraft"), Some("x500"));
    assert_eq!(arg(&sidecar.args, "--truth-addr"), Some("127.0.0.1:19580"));
    // Aviate's x500 drives no gimbal outputs.
    assert_eq!(arg(&sidecar.args, "--gimbal"), None);
    let fc = &stages[1].spec;
    assert_eq!(fc.name, "flight-controller");
    assert!(fc.program.ends_with("target/debug/sitl-jsbsim-x500"));
    assert_eq!(fc.cwd.as_deref(), Some(aviate.as_path()));
    // The instance selects the sidecar port (4560 + instance) and the XIL
    // ports in the FC.
    assert_eq!(env(&fc.env, "AVIATE_SITL_INSTANCE"), Some("20"));
    assert_eq!(
        env(&fc.env, "AVIATE_TELEMETRY_ENDPOINT"),
        Some("127.0.0.1:14720")
    );
    // The origin is where the sidecar stands the vehicle.
    let origin = env(&fc.env, "AVIATE_GNSS_ORIGIN").unwrap();
    let fields: Vec<&str> = origin.split(',').collect();
    assert_eq!(fields[0], arg(&sidecar.args, "--latitude-deg").unwrap());
    assert_eq!(fields[1], arg(&sidecar.args, "--longitude-deg").unwrap());
    assert_eq!(fields[2], arg(&sidecar.args, "--site-elevation-m").unwrap());
}

#[test]
fn missing_artifacts_name_their_fix() {
    let (root, aviate, jsbsim) = scaffold("missing");
    std::fs::remove_file(aviate.join("target/debug/sitl-jsbsim-x500")).unwrap();
    match plan_with_dirs(&context(&root, 0), &aviate, &jsbsim) {
        Err(XtaskError::MissingArtifact { hint, .. }) => {
            assert!(
                hint.contains("cargo build --bin sitl-jsbsim-x500"),
                "{hint}"
            );
        }
        Err(other) => panic!("expected a missing FC, got {other}"),
        Ok(_) => panic!("planned without the FC"),
    }
    assert!(matches!(
        plan_with_dirs(&context(&root, 0), &root.join("nowhere"), &jsbsim),
        Err(XtaskError::MissingArtifact {
            what: "Aviate checkout",
            ..
        })
    ));
    assert!(matches!(
        plan_with_dirs(&context(&root, 30), &aviate, &jsbsim),
        Err(XtaskError::Usage { .. })
    ));
}

#[test]
fn a_fleet_moves_the_vehicle_and_the_fc_origin_together() {
    let (root, aviate, jsbsim) = scaffold("spawn");
    let ctx = context(&root, 3);
    let mut stages = plan_with_dirs(&ctx, &aviate, &jsbsim).unwrap();
    let point = Placement {
        latitude_deg: 40.578,
        longitude_deg: -74.737,
        elevation_m: 61.25,
        heading_deg: 29.5,
    };
    AviateJsbsim.spawn(&ctx, &point, &mut stages).unwrap();
    let args = &stages[0].spec.args;
    assert_eq!(arg(args, "--latitude-deg"), Some("40.5780000"));
    assert_eq!(arg(args, "--site-elevation-m"), Some("61.25"));
    let origins: Vec<&str> = stages[1]
        .spec
        .env
        .iter()
        .filter(|(key, _)| key == "AVIATE_GNSS_ORIGIN")
        .map(|(_, value)| value.as_str())
        .collect();
    assert_eq!(origins, ["40.5780000,-74.7370000,61.25"]);
}

#[test]
fn the_host_meets_the_fc_on_the_instance_links() {
    let ctx = context(&scratch("host"), 20);
    let host = AviateJsbsim.host_env(&ctx);
    assert_eq!(env(&host, "PILOTAGE_AVIATE_ADDR"), Some("127.0.0.1:14720"));
    assert_eq!(
        env(&host, "PILOTAGE_AVIATE_FC_ADDR"),
        Some("127.0.0.1:20320")
    );
    // Aviate states system id 1 whatever the instance: the host default.
    assert_eq!(env(&host, "PILOTAGE_AVIATE_SYSTEM_ID"), None);
    assert_eq!(env(&host, "PILOTAGE_AVIATE_TRUTH"), Some("host-lane"));
    assert_eq!(
        env(&host, "PILOTAGE_SIM_TRUTH_ADDR"),
        Some("127.0.0.1:19580")
    );
    assert!(env(&host, "GZ_IP").is_none());
}

#[test]
fn instances_claim_their_own_ports_so_sessions_run_side_by_side() {
    let claims = |instance| AviateJsbsim.claims(&context(&scratch("claims"), instance));
    let ports = |claims: &[Claim]| {
        claims
            .iter()
            .map(|claim| match claim {
                Claim::Tcp { port, .. } | Claim::Udp { port, .. } => *port,
                other => panic!("a machine-wide claim {other:?}"),
            })
            .collect::<Vec<_>>()
    };
    let first = ports(&claims(20));
    assert!(first.contains(&4580) && first.contains(&20_320));
    assert!(first.contains(&14_720) && first.contains(&19_580));
    let second = ports(&claims(21));
    assert!(first.iter().all(|port| !second.contains(port)));
    assert_eq!(command_port(1), 20_016);
}
