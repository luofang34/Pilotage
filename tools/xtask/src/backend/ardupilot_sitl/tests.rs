#![allow(clippy::expect_used, clippy::panic)]

use super::{ArduPilotSitl, MAVLINK_PORT, plan_with_ardupilot_dir};
use crate::backend::{SessionContext, SimBackend};
use crate::cli::Profile;
use crate::error::XtaskError;

fn context(tag: &str) -> SessionContext {
    let root = std::env::temp_dir().join(format!("ardupilot-plan-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("scaffold");
    SessionContext {
        repo_root: root.clone(),
        host_port: 4433,
        viewer_port: 8080,
        profile: Profile::Simulation,
        log_dir: root.join("logs"),
        lan: false,
    }
}

#[test]
fn a_missing_binary_names_the_build_commands() {
    let ctx = context("missing");
    let refusal = plan_with_ardupilot_dir(&ctx, &ctx.repo_root.join("ardupilot"));
    let Err(XtaskError::MissingArtifact { hint, .. }) = refusal else {
        panic!("expected a missing artifact, got {refusal:?}");
    };
    assert!(hint.contains("./waf copter"));
    assert!(hint.contains("-no_fixup_chains"));
    assert!(!hint.contains("conda"));
}

#[test]
fn the_fc_sends_to_the_endpoint_the_host_listens_on() {
    let ctx = context("plan");
    let ardupilot = ctx.repo_root.join("ardupilot");
    for file in [
        "build/sitl/bin/arducopter",
        "Tools/autotest/default_params/copter.parm",
    ] {
        let path = ardupilot.join(file);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(&path, b"").expect("file");
    }
    let stages = plan_with_ardupilot_dir(&ctx, &ardupilot).expect("plan");
    let [stage] = stages.as_slice() else {
        panic!("one stage");
    };
    let serial = format!("udpclient:127.0.0.1:{MAVLINK_PORT}");
    assert!(stage.spec.args.contains(&serial));
    let host_env = ArduPilotSitl.host_env(&ctx);
    assert!(host_env.contains(&(
        "PILOTAGE_ARDUPILOT_ADDR".to_owned(),
        format!("127.0.0.1:{MAVLINK_PORT}")
    )));
    assert_eq!(ArduPilotSitl.host_adapter(), "ardupilot");
}

#[test]
fn plan_refuses_non_simulation_profiles() {
    let mut ctx = context("profile");
    ctx.profile = Profile::Physical;
    assert!(matches!(
        ArduPilotSitl.plan(&ctx),
        Err(XtaskError::Usage { .. })
    ));
}
