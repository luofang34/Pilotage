#![allow(clippy::expect_used, clippy::panic)]

use super::resolve;
use crate::aircraft::{AirData, AircraftState, Attitude, EstimateQuality, Kinematics, Stamped};
use crate::signal::{FreshnessPolicy, SignalStatus};
use pilotage_frames::Quat;

pub(crate) fn flying_state() -> AircraftState {
    AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat: Quat::IDENTITY,
                rates_rps: [0.0, 0.0, 0.05],
            }),
            age_ms: Some(50.0),
        },
        kinematics: Stamped {
            data: Some(Kinematics {
                pos_ned_m: [100.0, 0.0, -304.8],
                vel_ned_mps: [10.0, 10.0, -2.0],
            }),
            age_ms: Some(100.0),
        },
        air: Stamped {
            data: Some(AirData {
                ias_mps: None,
                baro_setting_hpa: Some(1013.25),
            }),
            age_ms: Some(100.0),
        },
        quality: EstimateQuality::Good,
        valid: crate::aircraft::ValidFlags {
            attitude: true,
            rates: true,
            position: true,
            velocity: true,
            ..Default::default()
        },
        ..AircraftState::default()
    }
}

#[test]
fn derives_display_units_from_si_ned() {
    let p = resolve(&flying_state(), &FreshnessPolicy::default());
    // 304.8 m up = 1000 ft.
    assert!((p.altitude.value_ft.value - 1000.0).abs() < 0.5);
    // 2 m/s climb ≈ 394 fpm.
    assert!((p.vsi_fpm.value - 393.7).abs() < 1.0);
    // 10,10 m/s ≈ 27.5 kt at 045°.
    assert!((p.gs_kt.value - 27.49).abs() < 0.1);
    assert!((p.track_rad.value - core::f32::consts::FRAC_PI_4).abs() < 1e-4);
    assert_eq!(p.roll_rad.status, SignalStatus::Valid);
}

#[test]
fn absent_airspeed_is_missing_not_zero() {
    let p = resolve(&flying_state(), &FreshnessPolicy::default());
    assert_eq!(p.ias_kt.status, SignalStatus::Missing);
    // Baro from the same group is still valid.
    assert_eq!(p.baro_hpa.status, SignalStatus::Valid);
}

#[test]
fn stale_attitude_is_flagged_stale() {
    let mut s = flying_state();
    s.attitude.age_ms = Some(1000.0);
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(p.roll_rad.status, SignalStatus::Stale);
    assert!(p.roll_rad.status.shows_value());
}

#[test]
fn dead_attitude_is_failed() {
    let mut s = flying_state();
    s.attitude.age_ms = Some(10_000.0);
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(p.roll_rad.status, SignalStatus::Failed);
    assert!(!p.roll_rad.status.shows_value());
}

#[test]
fn source_invalidity_beats_freshness() {
    let mut s = flying_state();
    s.valid.attitude = false;
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(p.roll_rad.status, SignalStatus::Failed);
    // The turn indication comes only from the dynamics group; with no
    // dynamics data it is Missing — body rates cannot leak in through
    // an attitude-flag path (DYN-01).
    assert_eq!(p.turn.rate_rps.status, SignalStatus::Missing);
}

#[test]
fn degraded_quality_taints_all_estimate_groups() {
    let mut s = flying_state();
    s.quality = EstimateQuality::Degraded;
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(p.roll_rad.status, SignalStatus::Degraded);
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Degraded);
}

#[test]
fn slow_track_is_meaningless_and_missing() {
    let mut s = flying_state();
    if let Some(kin) = s.kinematics.data.as_mut() {
        kin.vel_ned_mps = [0.1, 0.1, 0.0];
    }
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(p.track_rad.status, SignalStatus::Missing);
}

#[test]
fn empty_state_resolves_all_missing() {
    let p = resolve(&AircraftState::default(), &FreshnessPolicy::default());
    assert_eq!(p.roll_rad.status, SignalStatus::Missing);
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Missing);
    assert_eq!(p.ias_kt.status, SignalStatus::Missing);
    assert_eq!(p.nav.status, SignalStatus::Missing);
    assert_eq!(p.wind.status, SignalStatus::Missing);
}

#[test]
fn excessive_skew_degrades_both_stamped_groups() {
    use crate::aircraft::{SnapshotCoherence, SnapshotMeta};
    let mut s = flying_state();
    s.snapshot = SnapshotMeta {
        generation: 7,
        coherence: SnapshotCoherence::ExcessiveSkew,
    };
    let p = resolve(&s, &FreshnessPolicy::default());
    // Each value stays individually usable (amber, shown) but the pair
    // must not present as one coherent aircraft state.
    assert_eq!(p.roll_rad.status, SignalStatus::Degraded);
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Degraded);
    assert_eq!(p.vsi_fpm.status, SignalStatus::Degraded);
    // Groups outside the stamped attitude/kinematics pair are untouched.
    assert_eq!(p.baro_hpa.status, SignalStatus::Valid);
}

#[test]
fn coherent_and_insufficient_snapshots_do_not_degrade() {
    use crate::aircraft::{SnapshotCoherence, SnapshotMeta};
    for coherence in [SnapshotCoherence::Coherent, SnapshotCoherence::Insufficient] {
        let mut s = flying_state();
        s.snapshot = SnapshotMeta {
            generation: 1,
            coherence,
        };
        let p = resolve(&s, &FreshnessPolicy::default());
        assert_eq!(p.roll_rad.status, SignalStatus::Valid, "{coherence:?}");
        assert_eq!(
            p.altitude.value_ft.status,
            SignalStatus::Valid,
            "{coherence:?}"
        );
    }
}

#[test]
fn skew_degradation_never_upgrades_a_worse_status() {
    use crate::aircraft::{SnapshotCoherence, SnapshotMeta};
    let mut s = flying_state();
    s.valid.attitude = false;
    s.snapshot = SnapshotMeta {
        generation: 1,
        coherence: SnapshotCoherence::ExcessiveSkew,
    };
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(p.roll_rad.status, SignalStatus::Failed);
}

// ---- VAL-01 fail-safe resolution ---------------------------------------------

#[test]
fn undeclared_trust_never_resolves_valid() {
    // Data present but neither quality nor validity declared: the
    // fail-safe defaults resolve Failed, not Valid.
    let s = AircraftState {
        attitude: flying_state().attitude,
        kinematics: flying_state().kinematics,
        ..AircraftState::default()
    };
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(p.roll_rad.status, SignalStatus::Failed);
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Failed);
}

#[test]
fn invalid_quaternion_never_reaches_attitude_geometry() {
    let mut s = flying_state();
    if let Some(att) = s.attitude.data.as_mut() {
        att.quat = Quat {
            w: f32::NAN,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
    }
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(p.roll_rad.status, SignalStatus::Failed);
    assert_eq!(p.roll_rad.value, 0.0, "quiet zero, not quaternion output");
    assert!(p.pitch_rad.value.is_finite());
    // Isolation: kinematics-derived signals are untouched by the
    // attitude fault.
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Valid);
}

#[test]
fn denormalized_quaternion_within_tolerance_still_displays() {
    let mut s = flying_state();
    if let Some(att) = s.attitude.data.as_mut() {
        att.quat = Quat {
            w: 1.01,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
    }
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(p.roll_rad.status, SignalStatus::Valid);
    assert!(p.roll_rad.value.abs() < 1e-6);
}

#[test]
fn derived_overflow_fails_instead_of_showing_infinity() {
    let mut s = flying_state();
    if let Some(kin) = s.kinematics.data.as_mut() {
        // Finite input whose unit conversion overflows f32.
        kin.pos_ned_m[2] = -f32::MAX;
    }
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Failed);
    assert_eq!(p.altitude.value_ft.value, 0.0);
}

#[test]
fn every_showable_output_is_finite_under_hostile_input() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut s = flying_state();
        if let Some(att) = s.attitude.data.as_mut() {
            att.rates_rps = [bad; 3];
        }
        if let Some(kin) = s.kinematics.data.as_mut() {
            kin.vel_ned_mps = [bad; 3];
        }
        if let Some(air) = s.air.data.as_mut() {
            air.baro_setting_hpa = Some(bad);
        }
        s.selections.heading_bug_rad = bad;
        s.selections.altitude_sel_m = Some(bad);
        let p = resolve(&s, &FreshnessPolicy::default());
        for (name, sig) in [
            ("roll", p.roll_rad),
            ("pitch", p.pitch_rad),
            ("heading", p.heading.value_rad),
            ("turn", p.turn.rate_rps),
            ("slip", p.slip_lat_mps2),
            ("ias", p.ias_kt),
            ("gs", p.gs_kt),
            ("alt", p.altitude.value_ft),
            ("vsi", p.vsi_fpm),
            ("track", p.track_rad),
            ("baro", p.baro_hpa),
        ] {
            assert!(
                !sig.status.shows_value() || sig.value.is_finite(),
                "{name} shows non-finite {} for {bad}",
                sig.value
            );
        }
        assert!(p.selections.heading_bug_rad.is_finite());
        assert_eq!(p.selections.altitude_sel_m, None);
    }
}

#[test]
fn unknown_nav_source_fails_the_group_and_clears_guidance() {
    let mut s = flying_state();
    s.nav = Stamped {
        data: Some(pilotage_state_navdata_unknown()),
        age_ms: Some(10.0),
    };
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(p.nav.status, SignalStatus::Failed);
    assert_eq!(p.nav.data.cdi_dots, 0.0, "guidance values cleared");
}

fn pilotage_state_navdata_unknown() -> crate::aircraft::NavData {
    crate::aircraft::NavData {
        source: crate::aircraft::NavSource::Unknown,
        course_rad: 1.0,
        cdi_dots: 1.5,
        fromto: crate::aircraft::NavFromTo::To,
        vdev_dots: None,
        dist_nm: None,
        course_reference: crate::heading::HeadingReference::SimLocalTrue,
    }
}

#[test]
fn multi_fault_priority_resolves_the_worst() {
    // Degraded quality + excessive skew + a validity flag off: Failed
    // (the flag) must win over both Degraded causes.
    use crate::aircraft::{SnapshotCoherence, SnapshotMeta};
    let mut s = flying_state();
    s.quality = EstimateQuality::Degraded;
    s.snapshot = SnapshotMeta {
        generation: 1,
        coherence: SnapshotCoherence::ExcessiveSkew,
    };
    s.valid.attitude = false;
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(p.roll_rad.status, SignalStatus::Failed);
    // Without the flag the two Degraded causes stay Degraded.
    s.valid.attitude = true;
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(p.roll_rad.status, SignalStatus::Degraded);
}

#[test]
fn unknown_coherence_degrades_stamped_groups() {
    use crate::aircraft::{SnapshotCoherence, SnapshotMeta};
    let mut s = flying_state();
    s.snapshot = SnapshotMeta {
        generation: 1,
        coherence: SnapshotCoherence::Unknown,
    };
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(p.roll_rad.status, SignalStatus::Degraded);
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Degraded);
}

#[test]
fn faults_are_reported_for_diagnostics() {
    let mut s = flying_state();
    if let Some(att) = s.attitude.data.as_mut() {
        att.quat.w = f32::NAN;
    }
    let p = resolve(&s, &FreshnessPolicy::default());
    assert_eq!(
        p.integrity.attitude,
        Some(crate::validate::GroupFault::NonFinite)
    );
}
