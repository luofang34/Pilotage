#![allow(clippy::expect_used, clippy::panic)]

use std::string::String;
use std::vec::Vec;

use pilotage_instrument_scene::{Cmd, SceneCmds, SceneWriter};
use pilotage_instrument_state::{
    NavData, NavFromTo, NavResolved, NavSource, PanelData, Sig, SignalStatus,
};

use super::draw_hsi;

/// Heading comes from the explicit SIM-declared independent sample —
/// the attitude quaternion still describes the same yaw, but nothing
/// derives heading from it (NAV-01).
fn heading_only(heading_rad: f32) -> PanelData {
    let state = pilotage_instrument_state::AircraftState {
        attitude: pilotage_instrument_state::Stamped {
            data: Some(pilotage_instrument_state::Attitude {
                quat: quat_yaw(heading_rad),
                rates_rps: [0.0; 3],
            }),
            age_ms: Some(10.0),
        },
        heading: pilotage_instrument_state::Stamped {
            data: Some(pilotage_instrument_state::HeadingSample {
                heading_rad,
                reference: pilotage_instrument_state::HeadingReference::SimLocalTrue,
            }),
            age_ms: Some(10.0),
        },
        quality: pilotage_instrument_state::EstimateQuality::Good,
        valid: pilotage_instrument_state::ValidFlags {
            attitude: true,
            rates: true,
            position: true,
            velocity: true,
            heading: true,
            ..Default::default()
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
    draw_hsi(data, None, &mut w).expect("panel fits buffer");
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

fn layer_texts(scene: &[u8], wanted: LayerId) -> Vec<(String, [f32; 3])> {
    let mut inside = false;
    let mut found = Vec::new();
    for command in SceneCmds::new(scene).expect("valid scene") {
        match command.expect("valid command") {
            Cmd::BeginLayer { layer } => inside = layer == wanted,
            Cmd::EndLayer { layer } if layer == wanted => inside = false,
            Cmd::Text {
                x, y, size, text, ..
            } if inside => found.push((String::from(text), [x, y, size])),
            _ => {}
        }
    }
    found
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
            course_reference: pilotage_instrument_state::HeadingReference::SimLocalTrue,
        },
        status: SignalStatus::Valid,
        course_rose_rad: Sig::with_status(0.35, SignalStatus::Valid),
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
    data.heading.value_rad = Sig::with_status(0.0, SignalStatus::Failed);
    let scene = render(&data);
    let labels = texts(&scene);
    assert!(labels.iter().any(|t| t == "HDG"), "HDG flag: {labels:?}");
    assert!(
        layer_texts(&scene, LayerId::Annunciation)
            .contains(&(String::from("HDG"), [240.0, 190.0, 20.0])),
        "HDG failure must be an annunciation"
    );
    assert!(
        labels.iter().any(|t| t == "---"),
        "readout dashes: {labels:?}"
    );
}

// ---- layer contract ----------------------------------------------------------

use pilotage_instrument_scene::{LayerId, validate_layers};

#[test]
fn scenes_are_layered_for_every_heading_status() {
    for status in [
        SignalStatus::Valid,
        SignalStatus::Degraded,
        SignalStatus::Stale,
        SignalStatus::Missing,
        SignalStatus::Failed,
    ] {
        let mut data = heading_only(0.0);
        data.heading.value_rad = Sig::with_status(0.0, status);
        let scene = render(&data);
        let report = validate_layers(&scene).expect("layered scene validates");
        for layer in [
            LayerId::Background,
            LayerId::Attitude,
            LayerId::Tapes,
            LayerId::Guidance,
            LayerId::Annunciation,
        ] {
            assert!(report.contains(layer), "{status:?} missing {layer:?}");
        }
    }
}

#[test]
fn degraded_navigation_cue_is_an_annunciation() {
    let mut data = heading_only(0.0);
    data.nav = NavResolved {
        data: NavData {
            source: NavSource::Gps,
            course_rad: 0.35,
            cdi_dots: -1.2,
            fromto: NavFromTo::To,
            vdev_dots: Some(0.4),
            dist_nm: Some(40.3),
            course_reference: pilotage_instrument_state::HeadingReference::SimLocalTrue,
        },
        status: SignalStatus::Degraded,
        course_rose_rad: Sig::with_status(0.35, SignalStatus::Degraded),
    };
    let scene = render(&data);
    let expected = (String::from("NAV"), [240.0, 250.0, 11.0]);
    assert!(
        layer_texts(&scene, LayerId::Annunciation).contains(&expected),
        "NAV cue must be above guidance"
    );
    assert!(
        !layer_texts(&scene, LayerId::Guidance).contains(&expected),
        "NAV cue must not share the guidance band"
    );
}

// ---- NAV-01: reference labelling, no fabricated rose, typed angles ------

#[test]
fn sim_declared_heading_is_labelled_sim() {
    let labels = texts(&render(&heading_only(0.0)));
    assert!(labels.iter().any(|t| t == "SIM"), "{labels:?}");
}

#[test]
fn magnetic_heading_is_labelled_mag() {
    let mut data = heading_only(0.0);
    data.heading.reference = pilotage_instrument_state::HeadingReference::Magnetic;
    let labels = texts(&render(&data));
    assert!(labels.iter().any(|t| t == "MAG"), "{labels:?}");
}

#[test]
fn missing_heading_leaves_no_plausible_rose() {
    let mut data = heading_only(0.0);
    data.heading.value_rad =
        Sig::with_status(0.0, pilotage_instrument_state::SignalStatus::Missing);
    let labels = texts(&render(&data));
    assert!(
        labels.iter().any(|t| t == "HDG"),
        "missing heading must flag, not freeze: {labels:?}"
    );
    for cardinal in ["N", "E", "S", "W"] {
        assert!(
            !labels.iter().any(|t| t == cardinal),
            "{cardinal} must not paint on a dead rose: {labels:?}"
        );
    }
}

#[test]
fn incompatible_bug_is_suppressed_with_its_flag() {
    let mut data = heading_only(0.0);
    data.selections.heading_bug_rad = 1.0;
    data.heading_bug_rose_rad =
        Sig::with_status(0.0, pilotage_instrument_state::SignalStatus::Failed);
    let labels = texts(&render(&data));
    assert!(
        labels.iter().any(|t| t == "HDG REF"),
        "suppressed bug must say why: {labels:?}"
    );
    assert!(
        !labels.iter().any(|t| t == "057°"),
        "raw bug number must not render: {labels:?}"
    );
}

#[test]
fn incompatible_course_shows_dashes_and_no_cdi() {
    let mut data = heading_only(0.0);
    data.nav = NavResolved {
        data: NavData {
            source: NavSource::Gps,
            course_rad: 0.35,
            cdi_dots: -1.2,
            fromto: NavFromTo::To,
            vdev_dots: Some(0.4),
            dist_nm: Some(40.3),
            course_reference: pilotage_instrument_state::HeadingReference::True,
        },
        status: SignalStatus::Valid,
        course_rose_rad: Sig::with_status(0.0, SignalStatus::Failed),
    };
    let labels = texts(&render(&data));
    assert!(
        labels.iter().any(|t| t == "---°"),
        "course box dashes: {labels:?}"
    );
    assert!(
        !labels.iter().any(|t| t == "020°"),
        "raw course must not render: {labels:?}"
    );
}
