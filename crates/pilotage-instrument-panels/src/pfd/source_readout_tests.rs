#![allow(clippy::expect_used, clippy::panic)]

use std::string::String;

use pilotage_instrument_state::{
    AircraftState, AirframeDisplayProfile, Candidate, FreshnessPolicy, IntegrityLevel, PanelData,
    ScalarMeasure, ScalarUnit, SourceEpoch, SourceId, SourceInputs, SourceMonitors, SourcePolicies,
    SourceStep, UnusualAttitudeState, resolve_with_sources,
};

use super::PfdConfig;
use super::tests::{render, texts};

fn air(src: u8, now: u64, kt: f32) -> Candidate<ScalarMeasure> {
    Candidate {
        source: SourceId(src),
        epoch: SourceEpoch(1),
        source_time_ms: now,
        receive_time_ms: now,
        sequence: now as u32,
        valid: true,
        integrity: IntegrityLevel::None,
        accuracy: 0.0,
        measurement: ScalarMeasure {
            value: kt,
            unit: ScalarUnit::Knots,
        },
    }
}

/// Resolves one monitored PFD frame with the given airspeed candidates.
fn frame(
    monitors: &mut SourceMonitors,
    unusual: &mut UnusualAttitudeState,
    policies: &SourcePolicies,
    airspeed: &[Candidate<ScalarMeasure>],
    now: u64,
) -> PanelData {
    let profile = AirframeDisplayProfile::simulator();
    let fresh = FreshnessPolicy::default();
    let state = AircraftState::default();
    let inputs = SourceInputs {
        airspeed,
        ..SourceInputs::default()
    };
    let step = SourceStep {
        inputs,
        policies,
        now_ms: now,
    };
    let (panel, _report) = resolve_with_sources(&state, &fresh, &profile, unusual, monitors, &step);
    panel
}

#[test]
fn rendered_value_and_source_label_share_one_source_and_switch_together() {
    let policies = SourcePolicies::simulator();
    let mut monitors = SourceMonitors::new();
    let mut unusual = UnusualAttitudeState::default();

    // Primary (100) is selected: the rendered value is 100 and the rendered
    // label is SRC1 — the other source's value never appears under this label.
    let panel = frame(
        &mut monitors,
        &mut unusual,
        &policies,
        &[air(1, 0, 100.0), air(2, 0, 200.0)],
        0,
    );
    let t = texts(&render(&panel, &PfdConfig::default()));
    assert!(
        t.contains(&String::from("100")),
        "primary value rendered: {t:?}"
    );
    assert!(
        t.contains(&String::from("SRC1")),
        "primary label rendered: {t:?}"
    );
    assert!(
        !t.contains(&String::from("200")) && !t.contains(&String::from("SRC2")),
        "the unselected source must not appear: {t:?}"
    );

    // Primary fails: the rendered value AND its label switch to the secondary
    // together — never 100 under SRC2 or 200 under SRC1.
    let down = Candidate {
        valid: false,
        ..air(1, 100, 100.0)
    };
    let panel = frame(
        &mut monitors,
        &mut unusual,
        &policies,
        &[down, air(2, 100, 200.0)],
        100,
    );
    let t = texts(&render(&panel, &PfdConfig::default()));
    assert!(t.contains(&String::from("200")), "value switched: {t:?}");
    assert!(t.contains(&String::from("SRC2")), "label switched: {t:?}");
    assert!(
        !t.contains(&String::from("100")) && !t.contains(&String::from("SRC1")),
        "the old value and label must not linger: {t:?}"
    );
}

#[test]
fn sustained_miscompare_is_visibly_annunciated() {
    let policies = SourcePolicies::simulator();
    let mut monitors = SourceMonitors::new();
    let mut unusual = UnusualAttitudeState::default();

    // Hold the disagreement past the persistence window.
    let mut panel = None;
    for now in [0u64, 500, 1000] {
        panel = Some(frame(
            &mut monitors,
            &mut unusual,
            &policies,
            &[air(1, now, 100.0), air(2, now, 200.0)],
            now,
        ));
    }
    let t = texts(&render(&panel.expect("stepped"), &PfdConfig::default()));
    assert!(
        t.contains(&String::from("IAS CMP")),
        "a sustained miscompare must be visibly annunciated: {t:?}"
    );
    assert!(
        t.contains(&String::from("SRC1")),
        "the ambiguity keeps and labels the primary: {t:?}"
    );
}
