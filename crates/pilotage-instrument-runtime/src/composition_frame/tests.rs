#![allow(clippy::expect_used, clippy::panic)]

use indicate_instrument_scene::{Cmd, SceneCmds};
use indicate_instrument_state::{AircraftState, SignalStatus, Stamped};

use super::age_state;
use crate::tests::{attitude_state, encoded_state_block, write_state};
use crate::{RenderStatus, Runtime};

fn panel_scene(runtime: &Runtime, slot: usize) -> Vec<u8> {
    let panel = runtime
        .composition_panel_outcomes()
        .get(slot)
        .expect("composition slot");
    let start = panel.scene_offset as usize;
    let end = start + panel.scene_len as usize;
    runtime.composition_scene()[start..end].to_vec()
}

fn altitude_state() -> AircraftState {
    let mut state = attitude_state();
    state.kinematics = Stamped {
        data: Some(indicate_instrument_state::Kinematics {
            pos_ned_m: [0.0, 0.0, -300.0],
            vel_ned_mps: [0.0; 3],
        }),
        age_ms: Some(10.0),
    };
    state
}

fn scene_texts(scene: &[u8]) -> Vec<String> {
    SceneCmds::new(scene)
        .expect("valid scene")
        .map(|command| command.expect("valid command"))
        .filter_map(|command| match command {
            Cmd::Text { text, .. } => Some(String::from(text)),
            _ => None,
        })
        .collect()
}

fn alert_stack(scene: &[u8]) -> Vec<String> {
    const STACK: &[&str] = &["ALT SRC", "NAV SRC", "TRN RATE", "ALRT FAIL", "MORE"];
    scene_texts(scene)
        .into_iter()
        .filter(|text| STACK.contains(&text.as_str()))
        .collect()
}

#[test]
fn a_composition_commits_all_panels_and_one_generation() {
    let mut runtime = Runtime::new();
    write_state(&mut runtime, &attitude_state());

    let outcome = runtime.render_composition(0, 100, true);
    assert_eq!(outcome.status, RenderStatus::Ok);
    assert_eq!(outcome.generation, 1);
    assert!(outcome.scene_len > 0);
    assert_eq!(runtime.composition_panel_outcomes().len(), 2);
    assert_eq!(runtime.generation[..2], [1, 1]);

    for panel in runtime.composition_panel_outcomes() {
        assert_eq!(panel.status, RenderStatus::Ok);
        assert!(panel.scene_len > 0);
        assert_eq!(panel.generation, 1);
        let start = panel.scene_offset as usize;
        let end = start + panel.scene_len as usize;
        assert!(SceneCmds::new(&runtime.composition_scene()[start..end]).is_ok());
    }
}

#[test]
fn elapsed_time_crosses_the_stale_and_failed_thresholds() {
    let profile = indicate_instrument_state::AirframeDisplayProfile::simulator();
    let policy = indicate_instrument_state::FreshnessPolicy::default();
    let mut unusual = indicate_instrument_state::UnusualAttitudeState::default();
    for (delta_ms, expected) in [
        (0, SignalStatus::Valid),
        (800, SignalStatus::Stale),
        (3_100, SignalStatus::Failed),
    ] {
        let mut state = attitude_state();
        age_state(&mut state, delta_ms);
        let data =
            indicate_instrument_state::resolve_stateful(&state, &policy, &profile, &mut unusual);
        assert_eq!(data.roll_rad.status, expected);
        assert_eq!(data.pitch_rad.status, expected);
    }
}

#[test]
fn quiet_input_drives_alerts_and_both_panels_from_one_step() {
    let mut runtime = Runtime::new();
    write_state(&mut runtime, &altitude_state());

    let fresh = runtime.render_composition(0, 100, true);
    assert_eq!(fresh.alerts.active_count, 0);
    let stale = runtime.render_composition(800, 900, true);
    assert_eq!(stale.alerts.active_count, 0);
    let failed = runtime.render_composition(3_100, 3_200, true);
    assert!(failed.alerts.active_count >= 1);

    let pfd = alert_stack(&panel_scene(&runtime, 0));
    let hsi = alert_stack(&panel_scene(&runtime, 1));
    assert!(pfd.contains(&String::from("ALT SRC")), "{pfd:?}");
    assert_eq!(pfd, hsi, "both panels must use one alert output");
}

#[test]
fn web_sequence_and_composition_transaction_are_semantically_equal() {
    for delta_ms in [0, 800, 3_100] {
        let now_ms = 100 + delta_ms;
        let state = altitude_state();

        let mut apple = Runtime::new();
        write_state(&mut apple, &state);
        let apple_outcome = apple.render_composition(delta_ms, now_ms, true);
        assert_eq!(apple_outcome.status, RenderStatus::Ok);
        let apple_scenes = [panel_scene(&apple, 0), panel_scene(&apple, 1)];

        let mut web_state = state;
        age_state(&mut web_state, delta_ms);
        let mut web = Runtime::new();
        web.state = encoded_state_block(&web_state);
        let web_alerts = web.step_alerts(now_ms, true);
        assert_eq!(apple_outcome.alerts, web_alerts);
        for (panel, expected_scene) in apple_scenes.iter().enumerate() {
            let outcome = web.render(panel as u32);
            assert_eq!(outcome.status, RenderStatus::Ok);
            assert_eq!(
                &web.scene()[..outcome.scene_len as usize],
                expected_scene.as_slice()
            );
        }
    }
}

#[test]
fn one_panel_failure_commits_no_generation_or_alert_state() {
    let mut runtime = Runtime::new();
    write_state(&mut runtime, &attitude_state());
    runtime.config[1] = vec![1, 0, 9];

    let outcome = runtime.render_composition(0, 100, true);
    assert_eq!(outcome.status, RenderStatus::ConfigInvalid);
    assert_eq!(outcome.scene_len, 0);
    assert_eq!(outcome.generation, 0);
    assert_eq!(runtime.generation[..2], [0, 0]);
    assert!(runtime.alert_output.is_none());
    for panel in runtime.composition_panel_outcomes() {
        assert_eq!(panel.status, RenderStatus::ConfigInvalid);
        assert_eq!(panel.scene_len, 0);
        assert_eq!(panel.generation, 0);
    }
}

#[test]
fn composition_and_panel_generations_wrap_together() {
    let mut runtime = Runtime::new();
    write_state(&mut runtime, &attitude_state());
    runtime.composition_generation = u32::MAX;
    runtime.generation[0] = u32::MAX;
    runtime.generation[1] = u32::MAX;

    let outcome = runtime.render_composition(0, 100, true);
    assert_eq!(outcome.status, RenderStatus::Ok);
    assert_eq!(outcome.generation, 0);
    assert_eq!(runtime.generation[..2], [0, 0]);
    assert!(
        runtime
            .composition_panel_outcomes()
            .iter()
            .all(|panel| panel.generation == 0)
    );
}
