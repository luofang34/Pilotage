#![allow(clippy::expect_used, clippy::panic)]

use super::{ArduPilotSitl, plan_with_ardupilot_dir};
use crate::backend::{SessionContext, SimBackend, SpawnPoint};
use crate::cli::Profile;
use crate::conflicts::Claim;
use crate::error::XtaskError;

fn context(tag: &str, instance: u16) -> SessionContext {
    let root = std::env::temp_dir().join(format!("ardupilot-plan-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("scaffold");
    let mut ctx = SessionContext::for_test(root.clone());
    ctx.log_dir = root.join("logs");
    ctx.instance = instance;
    ctx
}

fn scaffolded(ctx: &SessionContext) -> std::path::PathBuf {
    let ardupilot = ctx.repo_root.join("ardupilot");
    for file in [
        "build/sitl/bin/arducopter",
        "Tools/autotest/default_params/copter.parm",
    ] {
        let path = ardupilot.join(file);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(&path, b"").expect("file");
    }
    ardupilot
}

#[test]
fn a_missing_binary_names_the_build_commands() {
    let ctx = context("missing", 0);
    let refusal = plan_with_ardupilot_dir(&ctx, &ctx.repo_root.join("ardupilot"));
    let Err(XtaskError::MissingArtifact { hint, .. }) = refusal else {
        panic!("expected a missing artifact, got {refusal:?}");
    };
    assert!(hint.contains("./waf copter"));
    assert!(hint.contains("-no_fixup_chains"));
    assert!(!hint.contains("conda"));
}

#[test]
fn each_instance_sends_to_the_endpoint_its_host_listens_on() {
    let ctx = context("instance", 2);
    let stages = plan_with_ardupilot_dir(&ctx, &scaffolded(&ctx)).expect("plan");
    let [stage] = stages.as_slice() else {
        panic!("one stage");
    };
    assert!(stage.spec.args.contains(&"-I2".to_owned()));
    assert!(
        stage
            .spec
            .args
            .contains(&"udpclient:127.0.0.1:14702".to_owned())
    );
    assert!(ArduPilotSitl.host_env(&ctx).contains(&(
        "PILOTAGE_ARDUPILOT_ADDR".to_owned(),
        "127.0.0.1:14702".to_owned()
    )));
}

#[test]
fn spawn_places_the_home_at_the_fleet_point() {
    let ctx = context("spawn", 1);
    let mut stages = plan_with_ardupilot_dir(&ctx, &scaffolded(&ctx)).expect("plan");
    let point = SpawnPoint {
        latitude_deg: 47.397_742,
        longitude_deg: 8.546_392,
    };
    ArduPilotSitl
        .spawn(&ctx, &point, &mut stages)
        .expect("spawn");
    let args = &stages[0].spec.args;
    let at = args.iter().position(|arg| arg == "--home").expect("--home");
    assert_eq!(args[at + 1], "47.3977420,8.5463920,488,0");
}

#[test]
fn claims_shift_with_the_instance() {
    let claims = ArduPilotSitl.claims(&context("claims", 3));
    let ports: Vec<u16> = claims
        .iter()
        .filter_map(|claim| match claim {
            Claim::Udp { port, .. } | Claim::Tcp { port, .. } => Some(*port),
            _ => None,
        })
        .collect();
    assert_eq!(ports, vec![14_703, 5_531, 5_792, 5_793]);
    assert!(
        !claims
            .iter()
            .any(|claim| matches!(claim, Claim::Process(_))),
        "several ArduPilot sessions may share the machine"
    );
}

#[test]
fn plan_refuses_non_simulation_profiles() {
    let mut ctx = context("profile", 0);
    ctx.profile = Profile::Physical;
    assert!(matches!(
        ArduPilotSitl.plan(&ctx),
        Err(XtaskError::Usage { .. })
    ));
}
