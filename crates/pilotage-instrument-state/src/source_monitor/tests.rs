#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use crate::source_compare::{
    AttitudeMeasure, Candidate, ComparisonState, FrameTag, IntegrityLevel, ScalarMeasure,
    ScalarUnit, SourceEpoch, SourceId,
};
use crate::{
    AirData, AircraftState, AirframeDisplayProfile, Attitude, EstimateQuality, FreshnessPolicy,
    Quat, SignalStatus, Stamped, UnusualAttitudeState, ValidFlags,
};
use pilotage_alerts::{
    AlertCondition, AlertContext, AlertEvent, AlertManager, AlertProfile, MiscompareFault,
};

const DEG: f32 = core::f32::consts::PI / 180.0;

/// A valid airspeed candidate carrying a distinct value in knots.
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

#[test]
fn selected_value_and_source_are_inseparable_and_switch_together() {
    let policies = SourcePolicies::simulator();
    let profile = AirframeDisplayProfile::simulator();
    let fresh = FreshnessPolicy::default();
    let state = AircraftState::default();
    let mut monitors = SourceMonitors::new();
    let mut unusual = UnusualAttitudeState::default();

    // Two airspeed sources carry distinct values; the primary is selected, so
    // the value read out is the primary's own.
    let inputs = SourceInputs {
        airspeed: &[air(1, 0, 100.0), air(2, 0, 200.0)],
        ..SourceInputs::default()
    };
    let step = SourceStep {
        inputs,
        policies: &policies,
        now_ms: 0,
    };
    let (panel, _report) =
        resolve_with_sources(&state, &fresh, &profile, &mut unusual, &mut monitors, &step);
    let sel = panel
        .sources
        .airspeed
        .selected
        .expect("a source is selected");
    assert_eq!(sel.source(), SourceId(1));
    assert_eq!(sel.value(), 100.0, "the value is the selected source's own");
    assert!(!panel.sources.airspeed.reverted);

    // The primary fails: the value and its source id move to the secondary
    // together — one cannot switch without the other.
    let inputs = SourceInputs {
        airspeed: &[
            Candidate {
                valid: false,
                ..air(1, 100, 100.0)
            },
            air(2, 100, 200.0),
        ],
        ..SourceInputs::default()
    };
    let step = SourceStep {
        inputs,
        policies: &policies,
        now_ms: 100,
    };
    let (panel, _report) =
        resolve_with_sources(&state, &fresh, &profile, &mut unusual, &mut monitors, &step);
    let sel = panel
        .sources
        .airspeed
        .selected
        .expect("the reverted source is selected");
    assert_eq!(sel.source(), SourceId(2), "the source id switched");
    assert_eq!(
        sel.value(),
        200.0,
        "the value switched to the secondary with its id"
    );
    assert!(panel.sources.airspeed.reverted);
}

/// A valid attitude candidate rotated `ang_deg` about the yaw axis.
fn att(src: u8, now: u64, ang_deg: f32) -> Candidate<AttitudeMeasure> {
    let half = ang_deg * DEG / 2.0;
    Candidate {
        source: SourceId(src),
        epoch: SourceEpoch(1),
        source_time_ms: now,
        receive_time_ms: now,
        sequence: now as u32,
        valid: true,
        integrity: IntegrityLevel::None,
        accuracy: 0.0,
        measurement: AttitudeMeasure {
            quat: Quat {
                w: libm::cosf(half),
                x: 0.0,
                y: 0.0,
                z: libm::sinf(half),
            },
            frame: FrameTag(1),
        },
    }
}

/// Packs the report's ALR-01 transitions into a fixed buffer and steps the
/// alert manager, returning its output.
fn feed_alerts(
    manager: &mut AlertManager,
    profile: &AlertProfile,
    report: &SourceMonitorReport,
    now: u64,
) -> pilotage_alerts::AlertOutput {
    let mut buf = [AlertEvent::AcknowledgeAll; 4];
    let mut n = 0;
    for event in report.transitions().into_iter().flatten() {
        buf[n] = event;
        n += 1;
    }
    manager.step(profile, &buf[..n], AlertContext::default(), now)
}

#[test]
fn resolve_with_sources_annunciates_selection_and_feeds_alr01() {
    let policies = SourcePolicies::simulator();
    let profile = AirframeDisplayProfile::simulator();
    let fresh = FreshnessPolicy::default();
    let state = AircraftState::default();
    let mut monitors = SourceMonitors::new();
    let mut unusual = UnusualAttitudeState::default();
    let mut alerts = AlertManager::new();
    let aprofile = AlertProfile::simulator();
    let miscompare_id = AlertCondition::Miscompare(MiscompareFault::Attitude).id();

    // Two attitude sources ~10 deg apart — beyond the 5 deg miscompare band —
    // held past the persistence window so the disagreement sustains.
    let mut last = None;
    for now in [0u64, 500, 1000] {
        let inputs = SourceInputs {
            attitude: &[att(1, now, 0.0), att(2, now, 10.0)],
            ..SourceInputs::default()
        };
        let step = SourceStep {
            inputs,
            policies: &policies,
            now_ms: now,
        };
        let (panel, report) =
            resolve_with_sources(&state, &fresh, &profile, &mut unusual, &mut monitors, &step);
        let out = feed_alerts(&mut alerts, &aprofile, &report, now);
        last = Some((panel, out));
    }

    let (panel, out) = last.expect("stepped at least once");
    assert_eq!(panel.sources.attitude.state, ComparisonState::Miscompare);
    assert_eq!(
        panel.sources.attitude.selected.map(|s| s.source()),
        Some(SourceId(1)),
        "the displayed value identifies its source; ambiguity keeps the primary"
    );
    assert!(!panel.sources.attitude.reverted);
    assert!(
        out.active().iter().any(|a| a.id == miscompare_id),
        "the typed miscompare transition reached ALR-01 and is active"
    );

    // The primary now fails: selection reverts to the secondary and the panel
    // annunciates the non-primary selection.
    let inputs = SourceInputs {
        attitude: &[
            Candidate {
                valid: false,
                ..att(1, 1500, 0.0)
            },
            att(2, 1500, 10.0),
        ],
        ..SourceInputs::default()
    };
    let step = SourceStep {
        inputs,
        policies: &policies,
        now_ms: 1500,
    };
    let (panel, _report) =
        resolve_with_sources(&state, &fresh, &profile, &mut unusual, &mut monitors, &step);
    assert_eq!(
        panel.sources.attitude.selected.map(|s| s.source()),
        Some(SourceId(2)),
        "a failed primary reverts to the secondary"
    );
    assert!(
        panel.sources.attitude.reverted,
        "the non-primary selection is annunciated"
    );
}

/// A base state carrying a valid airspeed and attitude, so a fall-back to it
/// would be visible if a monitored function failed to fail closed.
fn base_state() -> AircraftState {
    AircraftState {
        air: Stamped {
            data: Some(AirData {
                ias_mps: Some(50.0),
                baro_setting_hpa: Some(1013.0),
            }),
            age_ms: Some(10.0),
        },
        attitude: Stamped {
            data: Some(Attitude {
                quat: Quat::IDENTITY,
                rates_rps: [0.0, 0.0, 0.0],
            }),
            age_ms: Some(10.0),
        },
        quality: EstimateQuality::Good,
        valid: ValidFlags {
            attitude: true,
            rates: true,
            ..ValidFlags::default()
        },
        ..AircraftState::default()
    }
}

/// A valid attitude candidate banked `deg` degrees right (roll about body x).
fn att_bank(src: u8, now: u64, deg: f32) -> Candidate<AttitudeMeasure> {
    let half = deg.to_radians() / 2.0;
    Candidate {
        source: SourceId(src),
        epoch: SourceEpoch(1),
        source_time_ms: now,
        receive_time_ms: now,
        sequence: now as u32,
        valid: true,
        integrity: IntegrityLevel::None,
        accuracy: 0.0,
        measurement: AttitudeMeasure {
            quat: Quat {
                w: libm::cosf(half),
                x: libm::sinf(half),
                y: 0.0,
                z: 0.0,
            },
            frame: FrameTag(1),
        },
    }
}

#[test]
fn all_failed_monitored_function_fails_closed() {
    let policies = SourcePolicies::simulator();
    let profile = AirframeDisplayProfile::simulator();
    let fresh = FreshnessPolicy::default();
    let state = base_state();
    let mut monitors = SourceMonitors::new();
    let mut unusual = UnusualAttitudeState::default();

    // Airspeed and attitude are monitored but every candidate is invalid.
    let bad_air = [Candidate {
        valid: false,
        ..air(1, 0, 150.0)
    }];
    let bad_att = [Candidate {
        valid: false,
        ..att_bank(1, 0, 10.0)
    }];
    let inputs = SourceInputs {
        airspeed: &bad_air,
        attitude: &bad_att,
        ..SourceInputs::default()
    };
    let step = SourceStep {
        inputs,
        policies: &policies,
        now_ms: 0,
    };
    let (panel, _report) =
        resolve_with_sources(&state, &fresh, &profile, &mut unusual, &mut monitors, &step);

    assert_eq!(
        panel.ias_kt.status,
        SignalStatus::Failed,
        "no usable airspeed source fails closed, never the stale base value"
    );
    assert_eq!(panel.roll_rad.status, SignalStatus::Failed);
    assert_eq!(panel.pitch_rad.status, SignalStatus::Failed);
}

#[test]
fn airspeed_respects_the_knots_unit() {
    let policies = SourcePolicies::simulator();
    let profile = AirframeDisplayProfile::simulator();
    let fresh = FreshnessPolicy::default();
    let state = AircraftState::default();
    let mut monitors = SourceMonitors::new();
    let mut unusual = UnusualAttitudeState::default();

    // `air` declares Knots; the displayed value must be those knots, not the
    // m/s misreading (150 * MPS_TO_KT ~ 291).
    let knots = [air(1, 0, 150.0)];
    let inputs = SourceInputs {
        airspeed: &knots,
        ..SourceInputs::default()
    };
    let step = SourceStep {
        inputs,
        policies: &policies,
        now_ms: 0,
    };
    let (panel, _report) =
        resolve_with_sources(&state, &fresh, &profile, &mut unusual, &mut monitors, &step);

    assert!(
        (panel.ias_kt.value - 150.0).abs() < 0.5,
        "knots rendered as knots: {}",
        panel.ias_kt.value
    );
    assert!(
        (panel.ias_kt.value - 150.0 * crate::units::MPS_TO_KT).abs() > 100.0,
        "not misread as meters per second"
    );
}

#[test]
fn selected_attitude_hysteresis_holds_across_frames() {
    let policies = SourcePolicies::simulator();
    let profile = AirframeDisplayProfile::simulator();
    let fresh = FreshnessPolicy::default();
    let state = AircraftState::default();
    let mut monitors = SourceMonitors::new();
    let mut unusual = UnusualAttitudeState::default();

    // Jitter bank around the 65-degree unusual-bank entry; the persistent
    // per-source presentation must engage once and hold (exit is 60 degrees),
    // never resetting and chattering each frame.
    let mut transitions = 0u32;
    let mut last = false;
    for i in 0..40u64 {
        let bank = if i % 2 == 0 { 65.4 } else { 64.6 };
        let att = [att_bank(1, i, bank)];
        let inputs = SourceInputs {
            attitude: &att,
            ..SourceInputs::default()
        };
        let step = SourceStep {
            inputs,
            policies: &policies,
            now_ms: i,
        };
        let (panel, _report) =
            resolve_with_sources(&state, &fresh, &profile, &mut unusual, &mut monitors, &step);
        if panel.presentation.high_bank != last {
            transitions = transitions.wrapping_add(1);
            last = panel.presentation.high_bank;
        }
    }
    assert_eq!(transitions, 1, "engages once, no cross-frame chatter");
    assert!(last, "still latched inside the hysteresis band");
}
