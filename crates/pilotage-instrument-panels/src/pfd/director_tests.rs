//! Director command-bar behavior (#261): bars from a fully valid,
//! engaged director only; disappearance under every degradation; mode
//! annunciation as part of the feature.
#![allow(clippy::expect_used, clippy::panic)]

use std::string::String;
use std::vec;
use std::vec::Vec;

use pilotage_instrument_scene::{Cmd, PaintMode, SceneCmds, SceneWriter};
use pilotage_instrument_state::{
    AircraftState, Attitude, FdEngagement, FdMode, FdSample, FreshnessPolicy, PanelData, Quat,
    Stamped, resolve,
};

use crate::{PfdConfig, draw_pfd};

fn with_director(engagement: FdEngagement, age_ms: Option<f32>) -> PanelData {
    let state = AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat: Quat::IDENTITY,
                rates_rps: [0.0; 3],
            }),
            age_ms: Some(10.0),
        },
        director: Stamped {
            data: age_ms.map(|_| FdSample {
                pitch_cmd_rad: 0.1,
                roll_cmd_rad: -0.3,
                mode: FdMode::Nav,
                engagement,
            }),
            age_ms,
        },
        quality: pilotage_instrument_state::EstimateQuality::Good,
        valid: pilotage_instrument_state::ValidFlags {
            attitude: true,
            rates: true,
            ..Default::default()
        },
        ..Default::default()
    };
    resolve(&state, &FreshnessPolicy::default())
}

fn render(data: &PanelData) -> Vec<u8> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut writer = SceneWriter::new(&mut buf).expect("fits");
    draw_pfd(data, &PfdConfig::default(), None, &mut writer).expect("draws");
    let used = writer.finish();
    buf[..used].to_vec()
}

/// Filled rects inside the Guidance band — the command bars.
fn guidance_fills(scene: &[u8]) -> usize {
    use pilotage_instrument_scene::LayerId;
    let mut in_guidance = false;
    let mut fills = 0;
    for cmd in SceneCmds::new(scene).expect("decodes") {
        match cmd.expect("well-formed") {
            Cmd::BeginLayer {
                layer: LayerId::Guidance,
            } => in_guidance = true,
            Cmd::EndLayer {
                layer: LayerId::Guidance,
            } => in_guidance = false,
            Cmd::Rect {
                mode: PaintMode::Fill,
                ..
            } if in_guidance => fills += 1,
            _ => {}
        }
    }
    fills
}

fn texts(scene: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for cmd in SceneCmds::new(scene).expect("decodes") {
        if let Cmd::Text { text, .. } = cmd.expect("well-formed") {
            out.push(String::from(text));
        }
    }
    out
}

#[test]
fn an_engaged_director_draws_both_bars_and_its_mode() {
    let scene = render(&with_director(FdEngagement::Engaged, Some(60.0)));
    assert_eq!(guidance_fills(&scene), 2, "dual-cue bars");
    assert!(texts(&scene).iter().any(|t| t == "FD NAV"));
}

#[test]
fn an_armed_director_annunciates_but_commands_nothing() {
    let scene = render(&with_director(FdEngagement::Armed, Some(60.0)));
    assert_eq!(guidance_fills(&scene), 0, "armed must not command");
    assert!(texts(&scene).iter().any(|t| t == "FD NAV"));
}

#[test]
fn a_disengaged_or_absent_director_leaves_the_band_empty() {
    for data in [
        with_director(FdEngagement::Off, Some(60.0)),
        with_director(FdEngagement::Engaged, None),
    ] {
        let scene = render(&data);
        assert_eq!(guidance_fills(&scene), 0);
        assert!(!texts(&scene).iter().any(|t| t.starts_with("FD")));
    }
}

#[test]
fn a_stale_command_disappears_rather_than_freezing() {
    // Directly degrade the resolved status: however the degradation
    // arrives (freshness, trust, integrity), the bars must vanish —
    // not dash, not freeze.
    let mut data = with_director(FdEngagement::Engaged, Some(60.0));
    data.director.status = pilotage_instrument_state::SignalStatus::Failed;
    let scene = render(&data);
    assert_eq!(guidance_fills(&scene), 0);
}

#[test]
fn bars_need_a_shown_attitude() {
    // The bars mean "fly toward the command" relative to the current
    // attitude; without one they must not paint.
    let mut data = with_director(FdEngagement::Engaged, Some(60.0));
    data.roll_rad.status = pilotage_instrument_state::SignalStatus::Missing;
    data.pitch_rad.status = pilotage_instrument_state::SignalStatus::Missing;
    let scene = render(&data);
    assert_eq!(guidance_fills(&scene), 0);
}

#[test]
fn the_unusual_attitude_tier_strips_the_bars() {
    // Recovery is flown to the horizon: under the declutter tier the
    // chevrons own the center field, and an engaged director's bars —
    // which would overlay them from a higher band — must vanish with
    // the rest of the competing symbology. The mode annunciation
    // stays: what commands is still worth knowing during recovery.
    let mut data = with_director(FdEngagement::Engaged, Some(60.0));
    data.roll_rad = pilotage_instrument_state::Sig::with_status(
        2.8,
        pilotage_instrument_state::SignalStatus::Valid,
    );
    data.pitch_rad = pilotage_instrument_state::Sig::with_status(
        -0.9,
        pilotage_instrument_state::SignalStatus::Valid,
    );
    data.presentation.unusual = true;
    let scene = render(&data);
    assert_eq!(
        guidance_fills(&scene),
        0,
        "bars must not fight the chevrons"
    );
}
