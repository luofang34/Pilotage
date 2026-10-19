#![allow(clippy::expect_used, clippy::panic)]

use super::{BetaflightSitl, READY_LINE, plan_with_betaflight_dir};
use crate::backend::{SessionContext, SimBackend, SpawnPoint};
use crate::cli::Profile;
use crate::conflicts::{Claim, Remedy};
use crate::error::XtaskError;
use crate::readiness::Readiness;

fn context(tag: &str, instance: u16) -> SessionContext {
    let root = std::env::temp_dir().join(format!("betaflight-plan-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("scaffold");
    let mut ctx = SessionContext::for_test(root.clone());
    ctx.log_dir = root.join("logs");
    ctx.instance = instance;
    ctx
}

fn scaffolded(ctx: &SessionContext) -> std::path::PathBuf {
    let betaflight = ctx.repo_root.join("betaflight");
    let elf = betaflight.join("obj/main/betaflight_SITL.elf");
    std::fs::create_dir_all(elf.parent().expect("parent")).expect("dirs");
    std::fs::write(&elf, b"").expect("file");
    betaflight
}

fn value_after<'a>(args: &'a [String], flag: &str) -> &'a str {
    let at = args.iter().position(|arg| arg == flag).expect(flag);
    &args[at + 1]
}

#[test]
fn a_missing_sitl_names_the_build_command() {
    let ctx = context("missing", 0);
    let refusal = plan_with_betaflight_dir(&ctx, &ctx.repo_root.join("betaflight"));
    let Err(XtaskError::MissingArtifact { hint, .. }) = refusal else {
        panic!("expected a missing artifact, got {refusal:?}");
    };
    assert!(hint.contains("make TARGET=SITL"));
    assert!(hint.contains("ENABLE_GAZEBO_BRIDGE=0"));
}

#[test]
fn the_sidecar_runs_the_sitl_with_the_repository_config_and_waits_for_flight() {
    let ctx = context("plan", 0);
    let stages = plan_with_betaflight_dir(&ctx, &scaffolded(&ctx)).expect("plan");
    let [stage] = stages.as_slice() else {
        panic!("one stage");
    };
    assert!(
        stage
            .spec
            .program
            .ends_with("target/release/pilotage-multirotor-fdm")
    );
    let args = &stage.spec.args;
    assert!(value_after(args, "--betaflight").ends_with("obj/main/betaflight_SITL.elf"));
    assert!(value_after(args, "--betaflight-config").ends_with("sim/betaflight/sitl.cli"));
    let home = crate::site::ZURICH.spawn_point(crate::site::Ground::Field, 1);
    assert_eq!(
        value_after(args, "--origin-lat"),
        format!("{:.7}", home.latitude_deg)
    );
    assert_eq!(value_after(args, "--origin-alt"), "488.00");
    assert_eq!(value_after(args, "--north"), "0");
    assert!(matches!(
        stage.readiness,
        Readiness::LogContains { needle, .. } if needle == READY_LINE
    ));
    assert_eq!(BetaflightSitl.host_adapter(), "betaflight");
    assert!(BetaflightSitl.host_env(&ctx).contains(&(
        "PILOTAGE_BETAFLIGHT_PROFILE".to_owned(),
        "simulation".to_owned()
    )));
}

#[test]
fn spawn_moves_the_model_frame_to_the_fleet_point_altitude_and_heading() {
    let ctx = context("spawn", 0);
    let mut stages = plan_with_betaflight_dir(&ctx, &scaffolded(&ctx)).expect("plan");
    let point = SpawnPoint {
        latitude_deg: 40.577_186_1,
        longitude_deg: -74.738_191_8,
        elevation_m: 56.3,
        heading_deg: 29.5,
    };
    BetaflightSitl
        .spawn(&ctx, &point, &mut stages)
        .expect("spawn");
    let args = &stages[0].spec.args;
    assert_eq!(value_after(args, "--origin-lat"), "40.5771861");
    assert_eq!(value_after(args, "--origin-lon"), "-74.7381918");
    assert_eq!(value_after(args, "--origin-alt"), "56.30");
    assert_eq!(value_after(args, "--heading-deg"), "29.50");
    assert_eq!(value_after(args, "--north"), "0");
    assert_eq!(value_after(args, "--east"), "0");
}

#[test]
fn only_one_betaflight_runs_per_machine() {
    // A fleet slot plans at any instance; the fixed-port claims are what
    // keep a second Betaflight off the machine.
    let ctx = context("instance", 7);
    assert!(plan_with_betaflight_dir(&ctx, &scaffolded(&ctx)).is_ok());
    let claims = BetaflightSitl.claims(&context("claims", 7));
    let ports: Vec<u16> = claims
        .iter()
        .filter_map(|claim| match claim {
            Claim::Udp { port, .. } | Claim::Tcp { port, .. } => Some(*port),
            _ => None,
        })
        .collect();
    assert_eq!(ports, vec![9_001, 9_002, 9_003, 9_004, 5_761, 5_768]);
    assert!(claims.iter().all(|claim| match claim {
        Claim::Udp { remedy, .. } | Claim::Tcp { remedy, .. } => *remedy == Remedy::StopOther,
        _ => true,
    }));
    assert!(
        claims
            .iter()
            .any(|claim| matches!(claim, Claim::Process("betaflight_SITL.elf")))
    );
}

#[test]
fn plan_refuses_non_simulation_profiles() {
    let mut ctx = context("profile", 0);
    ctx.profile = Profile::Physical;
    assert!(matches!(
        BetaflightSitl.plan(&ctx),
        Err(XtaskError::Usage { .. })
    ));
}

#[test]
fn the_fc_waits_the_session_ready_timeout() {
    let mut ctx = context("ready", 0);
    ctx.ready_timeout_s = 333;
    let stages = plan_with_betaflight_dir(&ctx, &scaffolded(&ctx)).expect("plan");
    assert_eq!(stages[0].readiness.timeout_s(), 333);
}
