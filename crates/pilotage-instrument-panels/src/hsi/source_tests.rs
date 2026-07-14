#![allow(clippy::expect_used, clippy::panic)]

use std::string::String;
use std::vec::Vec;

use pilotage_instrument_scene::{Cmd, SceneCmds, SceneWriter};
use pilotage_instrument_state::{
    AircraftState, AirframeDisplayProfile, Candidate, FreshnessPolicy, HeadingMeasure,
    HeadingReference, IntegrityLevel, PanelData, SourceEpoch, SourceId, SourceInputs,
    SourceMonitors, SourcePolicies, SourceStep, UnusualAttitudeState, resolve_with_sources,
};

fn s(text: &str) -> String {
    String::from(text)
}

/// A heading candidate at `deg` degrees, magnetic.
fn hdg(src: u8, now: u64, deg: f32) -> Candidate<HeadingMeasure> {
    Candidate {
        source: SourceId(src),
        epoch: SourceEpoch(1),
        source_time_ms: now,
        receive_time_ms: now,
        sequence: now as u32,
        valid: true,
        integrity: IntegrityLevel::None,
        accuracy: 0.0,
        measurement: HeadingMeasure {
            heading_rad: deg.to_radians(),
            reference: HeadingReference::Magnetic,
        },
    }
}

fn frame(
    monitors: &mut SourceMonitors,
    unusual: &mut UnusualAttitudeState,
    policies: &SourcePolicies,
    heading: &[Candidate<HeadingMeasure>],
    now: u64,
) -> PanelData {
    let profile = AirframeDisplayProfile::simulator();
    let fresh = FreshnessPolicy::default();
    let state = AircraftState::default();
    let step = SourceStep {
        inputs: SourceInputs {
            heading,
            ..SourceInputs::default()
        },
        policies,
        now_ms: now,
    };
    resolve_with_sources(&state, &fresh, &profile, unusual, monitors, &step).0
}

fn texts(data: &PanelData) -> Vec<String> {
    let mut buf = std::vec![0u8; 32 * 1024];
    let mut writer = SceneWriter::new(&mut buf).expect("buffer fits");
    super::draw_hsi(data, None, &mut writer).expect("panel fits buffer");
    let len = writer.finish();
    SceneCmds::new(&buf[..len])
        .expect("valid scene")
        .map(|c| c.expect("valid command"))
        .filter_map(|c| match c {
            Cmd::Text { text, .. } => Some(String::from(text)),
            _ => None,
        })
        .collect()
}

#[test]
fn heading_box_value_and_label_share_one_source_and_switch_together() {
    let policies = SourcePolicies::simulator();
    let mut monitors = SourceMonitors::new();
    let mut unusual = UnusualAttitudeState::default();

    // Primary heading (030) selected: the heading box reads 030 and the label
    // names SRC1; the secondary heading (090) never appears under this label.
    let up = [hdg(1, 0, 30.0), hdg(2, 0, 90.0)];
    let t = texts(&frame(&mut monitors, &mut unusual, &policies, &up, 0));
    assert!(
        t.contains(&s("030°")) && t.contains(&s("HDG1")),
        "authoritative heading and its label are the primary: {t:?}"
    );
    assert!(
        !t.contains(&s("090°")) && !t.contains(&s("HDG2")),
        "the box can never show the unselected source: {t:?}"
    );

    // Primary fails: heading box value AND label switch to the secondary.
    let down = [
        Candidate {
            valid: false,
            ..hdg(1, 100, 30.0)
        },
        hdg(2, 100, 90.0),
    ];
    let t = texts(&frame(&mut monitors, &mut unusual, &policies, &down, 100));
    assert!(
        t.contains(&s("090°")) && t.contains(&s("HDG2")),
        "heading value and label switched together: {t:?}"
    );
    assert!(
        !t.contains(&s("030°")) && !t.contains(&s("HDG1")),
        "the old heading and label must not linger: {t:?}"
    );
}

#[test]
fn heading_sustained_miscompare_is_annunciated() {
    let policies = SourcePolicies::simulator();
    let mut monitors = SourceMonitors::new();
    let mut unusual = UnusualAttitudeState::default();
    let mut last = Vec::new();
    for now in [0u64, 500, 1000] {
        let up = [hdg(1, now, 30.0), hdg(2, now, 90.0)];
        last = texts(&frame(&mut monitors, &mut unusual, &policies, &up, now));
    }
    assert!(
        last.contains(&s("HDG CMP")),
        "miscompare annunciated: {last:?}"
    );
    assert!(
        last.contains(&s("HDG1")),
        "still names the retained primary: {last:?}"
    );
}
