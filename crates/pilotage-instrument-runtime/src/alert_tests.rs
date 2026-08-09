//! ALR-01 integration proofs: one manager output feeds every panel, and
//! primary-data flags never depend on the alerting path.

#![allow(clippy::expect_used, clippy::panic)]

use indicate_instrument_scene::SceneCmds;
use indicate_instrument_state::{AircraftState, Stamped};

use crate::tests::{attitude_state, write_state};
use crate::{RenderStatus, Runtime};

fn failed_alt_state() -> AircraftState {
    let mut state = attitude_state();
    state.kinematics = Stamped {
        data: Some(indicate_instrument_state::Kinematics {
            pos_ned_m: [0.0, 0.0, -300.0],
            vel_ned_mps: [0.0; 3],
        }),
        age_ms: Some(10.0),
    };
    state.valid.position = false;
    state.valid.velocity_horizontal = false;
    state.valid.velocity_vertical = false;
    state
}

fn committed_scene_texts(runtime: &Runtime, len: usize) -> Vec<String> {
    let scene = &runtime.scene[..len];
    SceneCmds::new(scene)
        .expect("valid scene")
        .map(|c| c.expect("valid command"))
        .filter_map(|c| match c {
            indicate_instrument_scene::Cmd::Text { text, .. } => Some(String::from(text)),
            _ => None,
        })
        .collect()
}

fn stack_only(texts: &[String]) -> Vec<String> {
    const STACK: &[&str] = &["ALT SRC", "NAV SRC", "TRN RATE", "ALRT FAIL", "MORE"];
    texts
        .iter()
        .filter(|t| STACK.contains(&t.as_str()))
        .cloned()
        .collect()
}

fn render_texts(runtime: &mut Runtime, panel: u32) -> Vec<String> {
    let outcome = runtime.render(panel);
    assert_eq!(outcome.status, RenderStatus::Ok);
    committed_scene_texts(runtime, outcome.scene_len as usize)
}

#[test]
fn one_alert_step_feeds_every_panel_the_same_semantic_state() {
    let mut runtime = Runtime::new();
    write_state(&mut runtime, &failed_alt_state());

    let outcome = runtime.step_alerts(1_000, true);
    assert_eq!(outcome.status, RenderStatus::Ok);
    assert!(outcome.active_count >= 1, "alt loss must assert an alert");
    assert!(!outcome.faulted, "healthy path is not faulted");

    let pfd = stack_only(&render_texts(&mut runtime, 0));
    let hsi = stack_only(&render_texts(&mut runtime, 1));
    assert!(pfd.contains(&String::from("ALT SRC")), "{pfd:?}");
    assert_eq!(pfd, hsi, "both panels consume the one cached AlertOutput");
}

#[test]
fn primary_flags_render_when_alerts_were_never_stepped() {
    let mut runtime = Runtime::new();
    write_state(&mut runtime, &failed_alt_state());

    let texts = render_texts(&mut runtime, 0);
    assert!(
        texts.contains(&String::from("ALT")),
        "ALT red X comes from resolved state, not the manager: {texts:?}"
    );
    assert!(
        stack_only(&texts).is_empty(),
        "no manager step, no alert stack: {texts:?}"
    );
}

#[test]
fn faulted_alerting_path_is_annunciated_and_flags_survive() {
    let mut runtime = Runtime::new();
    write_state(&mut runtime, &failed_alt_state());

    let outcome = runtime.step_alerts(1_000, false);
    assert!(outcome.faulted, "monitor fault must mark the output");

    let texts = render_texts(&mut runtime, 0);
    assert!(texts.contains(&String::from("ALRT FAIL")), "{texts:?}");
    assert!(
        texts.contains(&String::from("ALT")),
        "primary flag independent of the faulted alerting path: {texts:?}"
    );
}
