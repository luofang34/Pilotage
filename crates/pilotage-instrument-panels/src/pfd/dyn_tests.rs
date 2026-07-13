#![allow(clippy::expect_used, clippy::panic)]

//! DYN-01 rendering proofs: the cue labels its basis, the pointer
//! saturates while the value survives, missing slip draws no centered
//! ball, and required failures are loud.

use std::string::String;

use pilotage_instrument_scene::{Cmd, PaintMode, SceneCmds};
use pilotage_instrument_state::{PanelData, Sig, SignalStatus, TurnBasis};

use super::tests::{PfdConfig, flying, render, texts};

fn with_turn(rate_dps: f32) -> PanelData {
    let mut data = flying();
    data.turn.rate_rps = Sig::with_status(rate_dps.to_radians(), SignalStatus::Valid);
    data.turn.basis = TurnBasis::HeadingRate;
    data
}

/// Every filled rect in the scene, as (x, w) pairs, for locating the
/// turn-rate bar at its known y band.
fn turn_bar(scene: &[u8]) -> Option<(f32, f32)> {
    for command in SceneCmds::new(scene).expect("valid scene") {
        if let Cmd::Rect { mode, x, y, w, h } = command.expect("valid command")
            && mode == PaintMode::Fill
            && (y - 337.0).abs() < 0.6
            && (h - 6.0).abs() < 0.6
        {
            return Some((x, w));
        }
    }
    None
}

#[test]
fn cue_labels_its_basis() {
    let all = texts(&render(&with_turn(2.0), &PfdConfig::default()));
    assert!(all.contains(&String::from("HDG")), "{all:?}");
}

#[test]
fn plus_minus_three_degrees_reach_the_reference_ticks_symmetrically() {
    let (x_pos, w_pos) = turn_bar(&render(&with_turn(3.0), &PfdConfig::default())).expect("bar");
    assert!((x_pos - 240.0).abs() < 0.6 && (w_pos - 62.0).abs() < 0.6);
    let (x_neg, w_neg) = turn_bar(&render(&with_turn(-3.0), &PfdConfig::default())).expect("bar");
    assert!((x_neg - 178.0).abs() < 0.6 && (w_neg - 62.0).abs() < 0.6);
}

#[test]
fn pointer_saturates_but_the_resolved_value_does_not() {
    let data = with_turn(20.0);
    let (_, width) = turn_bar(&render(&data, &PfdConfig::default())).expect("bar");
    assert!(
        (width - 73.0).abs() < 0.6,
        "pointer clamps at the scale edge"
    );
    assert!(
        (data.turn.rate_rps.value - 20.0f32.to_radians()).abs() < 1e-6,
        "the monitored value keeps the over-range rate"
    );
}

#[test]
fn missing_turn_flags_trn_when_the_profile_requires_a_cue() {
    let mut data = flying();
    data.require_dynamics_cue = true;
    let all = texts(&render(&data, &PfdConfig::default()));
    assert!(all.contains(&String::from("TRN")), "{all:?}");
    data.require_dynamics_cue = false;
    let all = texts(&render(&data, &PfdConfig::default()));
    assert!(!all.contains(&String::from("TRN")), "{all:?}");
}

#[test]
fn missing_slip_never_draws_a_centered_ball() {
    let data = with_turn(0.0);
    let scene = render(&data, &PfdConfig::default());
    let balls: usize = SceneCmds::new(&scene)
        .expect("valid scene")
        .filter(|command| {
            matches!(
                command.as_ref().expect("valid command"),
                Cmd::Circle { cy, .. } if (cy - 354.0).abs() < 0.6
            )
        })
        .count();
    assert_eq!(balls, 0, "no ball without a slip input");
    let all = texts(&scene);
    assert!(
        all.contains(&String::from("SLIP")),
        "required cue flags the absence: {all:?}"
    );
}

#[test]
fn slip_ball_deflects_opposite_the_lateral_force_symmetrically() {
    let mut data = with_turn(0.0);
    data.slip_lat_mps2 = Sig::with_status(1.0, SignalStatus::Valid);
    let scene = render(&data, &PfdConfig::default());
    let cx_right_force = ball_x(&scene).expect("ball");
    data.slip_lat_mps2 = Sig::with_status(-1.0, SignalStatus::Valid);
    let cx_left_force = ball_x(&render(&data, &PfdConfig::default())).expect("ball");
    assert!(
        cx_right_force < 240.0 && cx_left_force > 240.0,
        "ball opposes the force: {cx_right_force} / {cx_left_force}"
    );
    assert!(((cx_right_force - 240.0) + (cx_left_force - 240.0)).abs() < 0.6);
}

fn ball_x(scene: &[u8]) -> Option<f32> {
    for command in SceneCmds::new(scene).expect("valid scene") {
        if let Cmd::Circle { cx, cy, .. } = command.expect("valid command")
            && (cy - 354.0).abs() < 0.6
        {
            return Some(cx);
        }
    }
    None
}
