//! ALR-01 integration proofs: one manager output feeds every panel, and
//! primary-data flags never depend on the alerting path.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_instrument_scene::SceneCmds;
use pilotage_instrument_state::{AircraftState, Stamped};

use crate::exports::InstrumentRuntime;
use crate::render_status::RenderStatus;
use crate::tests::{attitude_state, encoded_state_block, unpack};

fn failed_alt_state() -> AircraftState {
    let mut state = attitude_state();
    state.kinematics = Stamped {
        data: Some(pilotage_instrument_state::Kinematics {
            pos_ned_m: [0.0, 0.0, -300.0],
            vel_ned_mps: [0.0; 3],
        }),
        age_ms: Some(10.0),
    };
    state.valid.position = false;
    state.valid.velocity = false;
    state
}

fn committed_scene_texts(rt: &InstrumentRuntime, len: usize) -> Vec<String> {
    let runtime = rt.runtime.as_ref().expect("initialized");
    let scene = &runtime.scene[..len];
    SceneCmds::new(scene)
        .expect("valid scene")
        .map(|c| c.expect("valid command"))
        .filter_map(|c| match c {
            pilotage_instrument_scene::Cmd::Text { text, .. } => Some(String::from(text)),
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

fn render_texts(rt: &mut InstrumentRuntime, panel: u32) -> Vec<String> {
    let packed = unpack(rt.render_result(panel));
    assert_eq!(packed.status, RenderStatus::Ok as u32);
    committed_scene_texts(rt, packed.scene_len as usize)
}

#[test]
fn one_alert_step_feeds_every_panel_the_same_semantic_state() {
    let mut rt = InstrumentRuntime::new();
    rt.init();
    let block = encoded_state_block(&failed_alt_state());
    rt.runtime
        .as_mut()
        .expect("initialized")
        .state
        .copy_from_slice(&block);

    let summary = rt.step_alerts(1_000, 1);
    assert_eq!(summary & 0xff, RenderStatus::Ok as u64);
    assert!((summary >> 8) & 0xff >= 1, "alt loss must assert an alert");
    assert_eq!((summary >> 16) & 1, 0, "healthy path is not faulted");

    let pfd = stack_only(&render_texts(&mut rt, 0));
    let hsi = stack_only(&render_texts(&mut rt, 1));
    assert!(pfd.contains(&String::from("ALT SRC")), "{pfd:?}");
    assert_eq!(pfd, hsi, "both panels consume the one cached AlertOutput");
}

#[test]
fn primary_flags_render_when_alerts_were_never_stepped() {
    let mut rt = InstrumentRuntime::new();
    rt.init();
    let block = encoded_state_block(&failed_alt_state());
    rt.runtime
        .as_mut()
        .expect("initialized")
        .state
        .copy_from_slice(&block);

    let texts = render_texts(&mut rt, 0);
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
    let mut rt = InstrumentRuntime::new();
    rt.init();
    let block = encoded_state_block(&failed_alt_state());
    rt.runtime
        .as_mut()
        .expect("initialized")
        .state
        .copy_from_slice(&block);

    let summary = rt.step_alerts(1_000, 0);
    assert_eq!((summary >> 16) & 1, 1, "monitor fault must mark the output");

    let texts = render_texts(&mut rt, 0);
    assert!(texts.contains(&String::from("ALRT FAIL")), "{texts:?}");
    assert!(
        texts.contains(&String::from("ALT")),
        "primary flag independent of the faulted alerting path: {texts:?}"
    );
}
