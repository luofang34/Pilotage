#![allow(clippy::expect_used, clippy::panic)]

use std::string::String;

use pilotage_instrument_state::units::{M_TO_FT, MPS_TO_KT};
use pilotage_instrument_state::{
    AircraftState, AirframeDisplayProfile, AltitudeClass, AttitudeMeasure, Candidate, FrameTag,
    FreshnessPolicy, GeoidModelId, IntegrityLevel, OriginId, PanelData, Quat, ScalarMeasure,
    ScalarUnit, SourceAltitude, SourceEpoch, SourceId, SourceInputs, SourceMonitors,
    SourcePolicies, SourceStep, UnusualAttitudeState, resolve_with_sources,
};

use super::PfdConfig;
use super::tests::{render, texts};

fn s(text: &str) -> String {
    String::from(text)
}

/// An airspeed candidate whose value renders as `kt` knots on the tape.
fn air_kt(src: u8, now: u64, kt: f32) -> Candidate<ScalarMeasure> {
    base(
        src,
        now,
        ScalarMeasure {
            value: kt / MPS_TO_KT,
            unit: ScalarUnit::MetersPerSecond,
        },
    )
}

/// An altitude candidate whose value renders as `ft` feet on the tape.
fn alt_ft(src: u8, now: u64, ft: f32) -> Candidate<SourceAltitude> {
    base(
        src,
        now,
        SourceAltitude {
            value_m: ft / M_TO_FT,
            class: AltitudeClass::BaroIndicated,
            origin: OriginId(0),
            model: GeoidModelId::UNDECLARED,
        },
    )
}

/// An attitude candidate banked `deg` degrees right (roll about body x).
fn att_bank(src: u8, now: u64, deg: f32) -> Candidate<AttitudeMeasure> {
    let half = deg.to_radians() / 2.0;
    base(
        src,
        now,
        AttitudeMeasure {
            quat: Quat {
                w: libm::cosf(half),
                x: libm::sinf(half),
                y: 0.0,
                z: 0.0,
            },
            frame: FrameTag(1),
        },
    )
}

fn base<M>(src: u8, now: u64, measurement: M) -> Candidate<M> {
    Candidate {
        source: SourceId(src),
        epoch: SourceEpoch(1),
        source_time_ms: now,
        receive_time_ms: now,
        sequence: now as u32,
        valid: true,
        integrity: IntegrityLevel::None,
        accuracy: 0.0,
        measurement,
    }
}

fn frame(
    monitors: &mut SourceMonitors,
    unusual: &mut UnusualAttitudeState,
    policies: &SourcePolicies,
    inputs: SourceInputs,
    now: u64,
) -> PanelData {
    let profile = AirframeDisplayProfile::simulator();
    let fresh = FreshnessPolicy::default();
    let state = AircraftState::default();
    let step = SourceStep {
        inputs,
        policies,
        now_ms: now,
    };
    resolve_with_sources(&state, &fresh, &profile, unusual, monitors, &step).0
}

#[test]
fn airspeed_tape_value_and_label_share_one_source_and_switch_together() {
    let policies = SourcePolicies::simulator();
    let mut monitors = SourceMonitors::new();
    let mut unusual = UnusualAttitudeState::default();

    // Primary (100 kt) selected: the tape reads 100 and the label names SRC1;
    // the secondary's value (200) never appears under this label.
    let up = [air_kt(1, 0, 100.0), air_kt(2, 0, 200.0)];
    let panel = frame(
        &mut monitors,
        &mut unusual,
        &policies,
        SourceInputs {
            airspeed: &up,
            ..SourceInputs::default()
        },
        0,
    );
    let t = texts(&render(&panel, &PfdConfig::default()));
    assert!(
        t.contains(&s("100")) && t.contains(&s("IAS1")),
        "authoritative tape value and its label are the primary: {t:?}"
    );
    assert!(
        !t.contains(&s("200")) && !t.contains(&s("IAS2")),
        "the tape can never show the unselected source's value or label: {t:?}"
    );

    // Primary fails: the authoritative tape value AND its label switch to the
    // secondary together — never 100 under IAS2 or 200 under IAS1.
    let down = [
        Candidate {
            valid: false,
            ..air_kt(1, 100, 100.0)
        },
        air_kt(2, 100, 200.0),
    ];
    let panel = frame(
        &mut monitors,
        &mut unusual,
        &policies,
        SourceInputs {
            airspeed: &down,
            ..SourceInputs::default()
        },
        100,
    );
    let t = texts(&render(&panel, &PfdConfig::default()));
    assert!(
        t.contains(&s("200")) && t.contains(&s("IAS2")),
        "the tape value and label both switched to the secondary: {t:?}"
    );
    assert!(
        !t.contains(&s("100")) && !t.contains(&s("IAS1")),
        "the old value and label must not linger: {t:?}"
    );
}

#[test]
fn airspeed_sustained_miscompare_is_annunciated() {
    let policies = SourcePolicies::simulator();
    let mut monitors = SourceMonitors::new();
    let mut unusual = UnusualAttitudeState::default();
    let mut panel = None;
    for now in [0u64, 500, 1000] {
        let up = [air_kt(1, now, 100.0), air_kt(2, now, 200.0)];
        panel = Some(frame(
            &mut monitors,
            &mut unusual,
            &policies,
            SourceInputs {
                airspeed: &up,
                ..SourceInputs::default()
            },
            now,
        ));
    }
    let t = texts(&render(&panel.expect("stepped"), &PfdConfig::default()));
    assert!(t.contains(&s("IAS CMP")), "miscompare annunciated: {t:?}");
    assert!(
        t.contains(&s("IAS1")),
        "still names the retained primary: {t:?}"
    );
}

#[test]
fn altitude_tape_value_and_label_share_one_source_and_switch_together() {
    let policies = SourcePolicies::simulator();
    let mut monitors = SourceMonitors::new();
    let mut unusual = UnusualAttitudeState::default();

    let up = [alt_ft(1, 0, 1000.0), alt_ft(2, 0, 2000.0)];
    let panel = frame(
        &mut monitors,
        &mut unusual,
        &policies,
        SourceInputs {
            altitude: &up,
            ..SourceInputs::default()
        },
        0,
    );
    let t = texts(&render(&panel, &PfdConfig::default()));
    assert!(
        t.contains(&s("1000")) && t.contains(&s("ALT1")),
        "authoritative altitude and label are the primary: {t:?}"
    );

    let down = [
        Candidate {
            valid: false,
            ..alt_ft(1, 100, 1000.0)
        },
        alt_ft(2, 100, 2000.0),
    ];
    let panel = frame(
        &mut monitors,
        &mut unusual,
        &policies,
        SourceInputs {
            altitude: &down,
            ..SourceInputs::default()
        },
        100,
    );
    let t = texts(&render(&panel, &PfdConfig::default()));
    assert!(
        t.contains(&s("2000")) && t.contains(&s("ALT2")),
        "altitude value and label switched together: {t:?}"
    );
    assert!(
        !t.contains(&s("1000")) && !t.contains(&s("ALT1")),
        "the old altitude and label must not linger: {t:?}"
    );
}

#[test]
fn attitude_horizon_value_and_label_share_one_source_and_switch_together() {
    let policies = SourcePolicies::simulator();
    let mut monitors = SourceMonitors::new();
    let mut unusual = UnusualAttitudeState::default();

    let up = [att_bank(1, 0, 20.0), att_bank(2, 0, 40.0)];
    let panel = frame(
        &mut monitors,
        &mut unusual,
        &policies,
        SourceInputs {
            attitude: &up,
            ..SourceInputs::default()
        },
        0,
    );
    assert!(
        (panel.roll_rad.value - 20.0_f32.to_radians()).abs() < 0.01,
        "authoritative bank is the primary's: {}",
        panel.roll_rad.value
    );
    let t = texts(&render(&panel, &PfdConfig::default()));
    assert!(t.contains(&s("ATT1")) && !t.contains(&s("ATT2")), "{t:?}");

    let down = [
        Candidate {
            valid: false,
            ..att_bank(1, 100, 20.0)
        },
        att_bank(2, 100, 40.0),
    ];
    let panel = frame(
        &mut monitors,
        &mut unusual,
        &policies,
        SourceInputs {
            attitude: &down,
            ..SourceInputs::default()
        },
        100,
    );
    assert!(
        (panel.roll_rad.value - 40.0_f32.to_radians()).abs() < 0.01,
        "authoritative bank switched to the secondary: {}",
        panel.roll_rad.value
    );
    let t = texts(&render(&panel, &PfdConfig::default()));
    assert!(t.contains(&s("ATT2")) && !t.contains(&s("ATT1")), "{t:?}");
}
