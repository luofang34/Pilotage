#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use crate::source_compare::{
    AttitudeMeasure, Candidate, ComparisonState, FrameTag, IntegrityLevel, SourceEpoch, SourceId,
};
use crate::{AircraftState, AirframeDisplayProfile, FreshnessPolicy, Quat, UnusualAttitudeState};
use pilotage_alerts::{
    AlertCondition, AlertContext, AlertEvent, AlertManager, AlertProfile, MiscompareFault,
};

const DEG: f32 = core::f32::consts::PI / 180.0;

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
        panel.sources.attitude.selected,
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
        panel.sources.attitude.selected,
        Some(SourceId(2)),
        "a failed primary reverts to the secondary"
    );
    assert!(
        panel.sources.attitude.reverted,
        "the non-primary selection is annunciated"
    );
}
