#![allow(clippy::expect_used, clippy::panic)]

use std::string::String;
use std::vec::Vec;

use pilotage_instrument_scene::{Cmd, SceneCmds, SceneWriter};
use pilotage_instrument_state::{
    NavData, NavFromTo, NavResolved, NavSource, PanelData, Sig, SignalStatus,
};

use super::draw_hsi;

fn heading_only(heading_rad: f32) -> PanelData {
    let state = pilotage_instrument_state::AircraftState {
        attitude: pilotage_instrument_state::Stamped {
            data: Some(pilotage_instrument_state::Attitude {
                quat: quat_yaw(heading_rad),
                rates_rps: [0.0; 3],
            }),
            age_ms: Some(10.0),
        },
        ..Default::default()
    };
    pilotage_instrument_state::resolve(
        &state,
        &pilotage_instrument_state::FreshnessPolicy::default(),
    )
}

fn quat_yaw(yaw: f32) -> pilotage_instrument_state::Quat {
    let h = yaw / 2.0;
    pilotage_instrument_state::Quat {
        w: libm::cosf(h),
        x: 0.0,
        y: 0.0,
        z: libm::sinf(h),
    }
}

fn render(data: &PanelData) -> Vec<u8> {
    let mut buf = std::vec![0u8; 32 * 1024];
    let mut w = SceneWriter::new(&mut buf).expect("fits");
    draw_hsi(data, &mut w).expect("panel fits buffer");
    let len = w.finish();
    buf.truncate(len);
    buf
}

fn texts(scene: &[u8]) -> Vec<String> {
    SceneCmds::new(scene)
        .expect("valid scene")
        .map(|c| c.expect("valid command"))
        .filter_map(|c| match c {
            Cmd::Text { text, .. } => Some(String::from(text)),
            _ => None,
        })
        .collect()
}

#[test]
fn north_heading_reads_360() {
    let scene = render(&heading_only(0.0));
    let labels = texts(&scene);
    assert!(
        labels.iter().any(|t| t == "360°"),
        "heading box: {labels:?}"
    );
    // Cardinal labels present and upright (as text commands).
    for cardinal in ["N", "E", "S", "W"] {
        assert!(labels.iter().any(|t| t == cardinal), "{cardinal} missing");
    }
}

#[test]
fn east_heading_reads_090() {
    let scene = render(&heading_only(core::f32::consts::FRAC_PI_2));
    let labels = texts(&scene);
    assert!(
        labels.iter().any(|t| t == "090°"),
        "heading box: {labels:?}"
    );
}

#[test]
fn gps_course_draws_cdi_and_course_box() {
    let mut data = heading_only(0.0);
    data.nav = NavResolved {
        data: NavData {
            source: NavSource::Gps,
            course_rad: 0.35,
            cdi_dots: -1.2,
            fromto: NavFromTo::To,
            vdev_dots: Some(0.4),
            dist_nm: Some(40.3),
        },
        status: SignalStatus::Valid,
    };
    let scene = render(&data);
    let labels = texts(&scene);
    assert!(labels.iter().any(|t| t == "020°"), "course box: {labels:?}");
    assert!(labels.iter().any(|t| t == "40.3"), "dist box: {labels:?}");
    assert!(
        labels.iter().any(|t| t == "V"),
        "vdev source tag: {labels:?}"
    );
}

#[test]
fn no_nav_source_shows_dashes_and_no_cdi() {
    let no_nav = render(&heading_only(0.0));
    let labels = texts(&no_nav);
    assert!(labels.iter().any(|t| t == "---°"));
    assert!(labels.iter().any(|t| t == "--.-"));
}

#[test]
fn failed_heading_renders_red_x() {
    let mut data = heading_only(0.0);
    data.heading_rad = Sig::with_status(0.0, SignalStatus::Failed);
    let scene = render(&data);
    let labels = texts(&scene);
    assert!(labels.iter().any(|t| t == "HDG"), "HDG flag: {labels:?}");
    assert!(
        labels.iter().any(|t| t == "---"),
        "readout dashes: {labels:?}"
    );
}
