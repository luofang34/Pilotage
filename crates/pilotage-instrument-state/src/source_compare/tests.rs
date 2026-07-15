#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use pilotage_alerts::{AlertCondition, AlertEvent, MiscompareFault};

mod functions;

/// A fresh, valid airspeed candidate whose times and sequence track `now`.
fn air(source: u8, now: u64, value: f32) -> Candidate<ScalarMeasure> {
    Candidate {
        source: SourceId(source),
        epoch: SourceEpoch(1),
        source_time_ms: now,
        receive_time_ms: now,
        sequence: now as u32,
        valid: true,
        integrity: IntegrityLevel::None,
        accuracy: 0.0,
        measurement: ScalarMeasure {
            value,
            unit: ScalarUnit::MetersPerSecond,
        },
    }
}

fn pol_air() -> AirframeSourcePolicy {
    AirframeSourcePolicy::simulator(MiscompareFault::Airspeed)
}

fn pol3() -> AirframeSourcePolicy {
    let mut priority = SourceList::new();
    priority.try_push(SourceId(1));
    priority.try_push(SourceId(2));
    priority.try_push(SourceId(3));
    AirframeSourcePolicy::new(SourcePolicyLimits {
        priority,
        agree_within: 2.5,
        miscompare_beyond: 5.0,
        skew_budget_ms: 50,
        max_age_ms: 500,
        sustain_ms: 1_000,
        return_stable_ms: 3_000,
        allow_reversion: true,
        allow_manual: true,
        use_integrity_tiebreak: false,
        use_accuracy_band: false,
    })
    .expect("valid three-source policy")
}

#[test]
fn agreeing_sources_select_primary_and_agree() {
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    let out = c.step(&[air(1, 0, 100.0), air(2, 0, 100.4)], &pol_air(), 0);
    assert_eq!(out.state, ComparisonState::Agree);
    assert_eq!(
        out.selected,
        Some(SourceId(1)),
        "displayed value identifies its source"
    );
    assert!(!out.reverted);
    assert_eq!(out.fault, None);
    assert_eq!(out.transition, None);
}

#[test]
fn transient_difference_does_not_sustain() {
    let p = pol_air();
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    let spike = c.step(&[air(1, 0, 100.0), air(2, 0, 110.0)], &p, 0);
    assert_eq!(spike.state, ComparisonState::Agree, "not yet sustained");
    let resolved = c.step(&[air(1, 500, 100.0), air(2, 500, 100.1)], &p, 500);
    assert_eq!(resolved.state, ComparisonState::Agree);
    assert_eq!(resolved.transition, None);
}

#[test]
fn sustained_difference_becomes_miscompare_and_annunciates() {
    let p = pol_air();
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    for t in [0u64, 500] {
        assert_eq!(
            c.step(&[air(1, t, 100.0), air(2, t, 110.0)], &p, t).state,
            ComparisonState::Agree,
        );
    }
    let sustained = c.step(&[air(1, 1000, 100.0), air(2, 1000, 110.0)], &p, 1000);
    assert_eq!(sustained.state, ComparisonState::Miscompare);
    assert_eq!(sustained.fault, Some(MiscompareFault::Airspeed));
    assert_eq!(
        sustained.transition,
        Some(AlertEvent::Assert(AlertCondition::Miscompare(
            MiscompareFault::Airspeed
        )))
    );
    assert_eq!(
        sustained.selected,
        Some(SourceId(1)),
        "ambiguity keeps the primary"
    );
    assert!(!sustained.reverted);
    let cleared = c.step(&[air(1, 1500, 100.0), air(2, 1500, 100.0)], &p, 1500);
    assert_eq!(cleared.state, ComparisonState::Agree);
    assert_eq!(
        cleared.transition,
        Some(AlertEvent::Clear(AlertCondition::Miscompare(
            MiscompareFault::Airspeed
        )))
    );
}

#[test]
fn two_source_disagreement_stays_ambiguous_without_integrity_evidence() {
    let p = pol_air();
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    for t in [0u64, 500, 1000, 1500] {
        let out = c.step(&[air(1, t, 100.0), air(2, t, 120.0)], &p, t);
        if t >= 1000 {
            assert_eq!(out.state, ComparisonState::Miscompare);
            assert_eq!(
                out.selected,
                Some(SourceId(1)),
                "never selects a peer by value"
            );
            assert!(!out.reverted);
        }
    }
}

#[test]
fn integrity_evidence_justifies_selecting_the_higher_integrity_source() {
    let p = pol_air();
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    for t in [0u64, 500, 1000] {
        let hi = Candidate {
            integrity: IntegrityLevel::High,
            ..air(2, t, 120.0)
        };
        let out = c.step(&[air(1, t, 100.0), hi], &p, t);
        if t >= 1000 {
            assert_eq!(out.state, ComparisonState::Miscompare);
            assert_eq!(
                out.selected,
                Some(SourceId(2)),
                "higher integrity breaks the tie"
            );
            assert!(out.reverted);
        }
    }
}

#[test]
fn every_source_failure_combination_selects_by_priority() {
    let p = pol3();
    for mask in 0u8..8 {
        let mut c = SourceComparator::new(MiscompareFault::Airspeed);
        let up = |i: u8| mask & (1 << (i - 1)) != 0;
        let mk = |i: u8| Candidate {
            valid: up(i),
            ..air(i, 0, 100.0)
        };
        let out = c.step(&[mk(1), mk(2), mk(3)], &p, 0);
        let expected = [1u8, 2, 3].into_iter().find(|&i| up(i));
        assert_eq!(out.selected, expected.map(SourceId), "mask {mask:03b}");
        let available = [1u8, 2, 3].into_iter().filter(|&i| up(i)).count();
        let expected_state = if available >= 2 {
            ComparisonState::Agree
        } else {
            ComparisonState::InsufficientSources
        };
        assert_eq!(out.state, expected_state, "mask {mask:03b}");
        let expect_reverted = expected.is_some() && expected.map(SourceId) != p.primary();
        assert_eq!(out.reverted, expect_reverted, "mask {mask:03b}");
    }
}

#[test]
fn stale_samples_are_never_compared() {
    let p = pol_air();
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    let stale = Candidate {
        receive_time_ms: 400,
        ..air(2, 1000, 130.0)
    };
    let out = c.step(&[air(1, 1000, 100.0), stale], &p, 1000);
    assert_eq!(
        out.state,
        ComparisonState::InsufficientSources,
        "stale peer excluded"
    );
    assert_eq!(out.selected, Some(SourceId(1)));
}

#[test]
fn skewed_samples_are_not_comparable() {
    let p = pol_air();
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    let skewed = Candidate {
        source_time_ms: 0,
        ..air(2, 100, 100.0)
    };
    let out = c.step(&[air(1, 100, 100.0), skewed], &p, 100);
    assert_eq!(
        out.state,
        ComparisonState::NotComparable,
        "acquisition skew exceeds the budget"
    );
    assert_eq!(out.selected, Some(SourceId(1)));
}

#[test]
fn epoch_change_resets_persistence_and_accepts_the_restart() {
    let p = pol_air();
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    for t in [0u64, 500, 1000] {
        c.step(&[air(1, t, 100.0), air(2, t, 120.0)], &p, t);
    }
    let restart = |src: u8, now: u64| Candidate {
        epoch: SourceEpoch(2),
        sequence: 0,
        ..air(src, now, 100.0)
    };
    let out = c.step(&[restart(1, 1500), restart(2, 1500)], &p, 1500);
    assert_eq!(
        out.state,
        ComparisonState::Agree,
        "persistence reset and the low-sequence restart is accepted"
    );
    assert_eq!(
        out.transition,
        Some(AlertEvent::Clear(AlertCondition::Miscompare(
            MiscompareFault::Airspeed
        )))
    );
    assert_eq!(out.selected, Some(SourceId(1)));
}

#[test]
fn reordered_samples_are_dropped() {
    let p = pol_air();
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    c.step(&[air(1, 100, 100.0), air(2, 100, 100.0)], &p, 100);
    let advanced = Candidate {
        sequence: 200,
        ..air(1, 200, 100.0)
    };
    let reordered = Candidate {
        sequence: 50,
        ..air(2, 200, 100.0)
    };
    let out = c.step(&[advanced, reordered], &p, 200);
    assert_eq!(
        out.state,
        ComparisonState::InsufficientSources,
        "reordered peer dropped"
    );
    assert_eq!(out.selected, Some(SourceId(1)));
    let out2 = c.step(&[air(1, 300, 100.0), air(2, 300, 100.0)], &p, 300);
    assert_eq!(
        out2.state,
        ComparisonState::Agree,
        "a genuinely newer sample is accepted"
    );
}

#[test]
fn miscompare_hysteresis_does_not_chatter() {
    let mut priority = SourceList::new();
    priority.try_push(SourceId(1));
    priority.try_push(SourceId(2));
    let p = AirframeSourcePolicy::new(SourcePolicyLimits {
        priority,
        agree_within: 2.5,
        miscompare_beyond: 5.0,
        skew_budget_ms: 50,
        max_age_ms: 500,
        sustain_ms: 0,
        return_stable_ms: 3_000,
        allow_reversion: true,
        allow_manual: false,
        use_integrity_tiebreak: false,
        use_accuracy_band: false,
    })
    .expect("valid policy");
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    let mut transitions = 0u32;
    let mut prev = ComparisonState::InsufficientSources;
    for i in 0..40u64 {
        let d = if i % 2 == 0 { 5.2 } else { 2.6 };
        let out = c.step(&[air(1, i, 100.0), air(2, i, 100.0 + d)], &p, i);
        if out.state != prev {
            transitions = transitions.wrapping_add(1);
            prev = out.state;
        }
    }
    assert_eq!(
        transitions, 1,
        "engages once; the in-band jitter never releases"
    );
    assert_eq!(prev, ComparisonState::Miscompare);
    let released = c.step(&[air(1, 100, 100.0), air(2, 100, 102.0)], &p, 100);
    assert_eq!(
        released.state,
        ComparisonState::Agree,
        "drops below the exit threshold"
    );
}

#[test]
fn manual_selection_is_honored_while_available() {
    let p = pol_air();
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    c.set_manual(Some(SourceId(2)));
    let out = c.step(&[air(1, 0, 100.0), air(2, 0, 100.0)], &p, 0);
    assert_eq!(out.selected, Some(SourceId(2)));
    assert!(out.manual);
    assert!(
        out.reverted,
        "a manual secondary is a non-primary selection"
    );
    let fail2 = Candidate {
        valid: false,
        ..air(2, 100, 100.0)
    };
    let auto = c.step(&[air(1, 100, 100.0), fail2], &p, 100);
    assert_eq!(
        auto.selected,
        Some(SourceId(1)),
        "automatic selection resumes"
    );
    assert!(!auto.manual);
    let back = c.step(&[air(1, 200, 100.0), air(2, 200, 100.0)], &p, 200);
    assert_eq!(
        back.selected,
        Some(SourceId(2)),
        "manual honored again on return"
    );
    assert!(back.manual);
}

#[test]
fn return_to_primary_cannot_chatter() {
    let p = pol_air();
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    let down = |t: u64| Candidate {
        valid: false,
        ..air(1, t, 100.0)
    };
    let reverted = c.step(&[down(0), air(2, 0, 100.0)], &p, 0);
    assert_eq!(reverted.selected, Some(SourceId(2)));
    assert!(reverted.reverted);
    let mut t = 500u64;
    for k in 0..6 {
        let primary = if k % 2 == 0 {
            air(1, t, 100.0)
        } else {
            down(t)
        };
        let out = c.step(&[primary, air(2, t, 100.0)], &p, t);
        assert_eq!(
            out.selected,
            Some(SourceId(2)),
            "no return while the primary flaps, t={t}"
        );
        t += 500;
    }
    let base = t;
    c.step(&[air(1, base, 100.0), air(2, base, 100.0)], &p, base);
    let returned = c.step(
        &[air(1, base + 3000, 100.0), air(2, base + 3000, 100.0)],
        &p,
        base + 3000,
    );
    assert_eq!(
        returned.selected,
        Some(SourceId(1)),
        "returns after stable availability"
    );
    assert!(!returned.reverted);
}

#[test]
fn stale_primary_does_not_strand_a_fresh_secondary() {
    // A stale primary must not anchor the comparison epoch: doing so would
    // reject a fresh, valid secondary of a different epoch as incoherent and
    // strand the display on the failed primary instead of reverting.
    let p = pol_air();
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    let stale_primary = Candidate {
        receive_time_ms: 0,
        source_time_ms: 0,
        ..air(1, 1000, 100.0)
    };
    let fresh_secondary = Candidate {
        epoch: SourceEpoch(2),
        ..air(2, 1000, 130.0)
    };
    let out = c.step(&[stale_primary, fresh_secondary], &p, 1000);
    assert_eq!(
        out.selected,
        Some(SourceId(2)),
        "reverts to the fresh secondary rather than stranding on the stale primary"
    );
    assert!(out.reverted);
}

#[test]
fn future_receive_time_is_not_treated_as_fresh() {
    // A receive stamp in the future must be rejected as invalid, not
    // saturated to age zero and accepted as maximally fresh.
    let p = pol_air();
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    let future_receive = Candidate {
        receive_time_ms: 2000,
        ..air(2, 1000, 100.0)
    };
    let out = c.step(&[air(1, 1000, 100.0), future_receive], &p, 1000);
    assert_eq!(
        out.state,
        ComparisonState::InsufficientSources,
        "a future-stamped peer is excluded, not compared"
    );
    assert_eq!(out.selected, Some(SourceId(1)));
    let mut c2 = SourceComparator::new(MiscompareFault::Airspeed);
    let present = c2.step(&[air(1, 1000, 100.0), air(2, 1000, 100.0)], &p, 1000);
    assert_eq!(
        present.state,
        ComparisonState::Agree,
        "a present-time peer at the same instant does compare"
    );
}
