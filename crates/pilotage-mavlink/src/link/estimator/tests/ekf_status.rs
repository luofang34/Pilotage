//! Authorization from ArduPilot's EKF_STATUS_REPORT (msg 193): the report
//! has no timestamp, so it is stamped at the newest accepted FC boot time
//! and grants the bounded-lag authorization the standard status does.

use std::sync::{Arc, Mutex};

use crate::codec::FcMessage;
use crate::link::LinkState;

use super::super::ekf_status_authorization;
use super::{QUALITY_DEGRADED, QUALITY_GOOD, QUALITY_UNUSABLE, apply, attitude, kinematics};

/// Attitude, both velocity bits, relative and absolute horizontal
/// position, absolute vertical position: a healthy GPS-aided EKF.
const HEALTHY: u16 = 1 | 2 | 4 | 8 | 16 | 32;

fn ekf_state() -> Arc<Mutex<LinkState>> {
    Arc::new(Mutex::new(LinkState {
        authorization_source: crate::AuthorizationSource::ArduPilotEkfStatus,
        standard_status_max_lag_ms: 300,
        maximum_inter_group_skew_ms: 300,
        ..LinkState::default()
    }))
}

fn ekf(flags: u16) -> FcMessage {
    FcMessage::EkfStatusReport { flags }
}

#[test]
fn healthy_flags_grant_full_authorization() {
    assert_eq!(ekf_status_authorization(HEALTHY), (0b1111, 2));
}

#[test]
fn uninitialized_filter_grants_nothing() {
    assert_eq!(ekf_status_authorization(HEALTHY | 1024), (0, 0));
}

#[test]
fn constant_position_mode_keeps_only_degraded_attitude() {
    assert_eq!(ekf_status_authorization(HEALTHY | 128), (0b0011, 1));
}

#[test]
fn gps_glitch_degrades_quality_but_keeps_validity() {
    assert_eq!(ekf_status_authorization(HEALTHY | 32_768), (0b1111, 1));
}

#[test]
fn report_stamped_at_the_high_water_authorizes_later_numerics() {
    let state = ekf_state();
    apply(&state, &[attitude(1_000), kinematics(1_000)]);
    apply(&state, &[ekf(HEALTHY)]);
    apply(&state, &[attitude(1_100), kinematics(1_100)]);

    let latest = state.lock().expect("lock");
    let status = latest.estimator_status.expect("the report is accepted");
    assert_eq!(status.time_boot_ms, 1_000, "stamped at the high-water mark");
    let attitude = latest.attitude.expect("attitude");
    let kinematics = latest.kinematics.expect("kinematics");
    assert_eq!(
        (attitude.valid_flags, attitude.quality),
        (0b1111, QUALITY_GOOD)
    );
    assert_eq!(
        (kinematics.valid_flags, kinematics.quality),
        (0b1111, QUALITY_GOOD)
    );
}

#[test]
fn numerics_beyond_the_lag_fail_closed() {
    let state = ekf_state();
    apply(&state, &[attitude(1_000)]);
    apply(&state, &[ekf(HEALTHY)]);
    apply(&state, &[attitude(1_400)]);

    let latest = state.lock().expect("lock");
    assert_eq!(latest.attitude.expect("attitude").quality, QUALITY_UNUSABLE);
}

#[test]
fn report_before_any_numeric_is_dropped() {
    let state = ekf_state();
    apply(&state, &[ekf(HEALTHY)]);
    assert!(state.lock().expect("lock").estimator_status.is_none());
}

#[test]
fn a_glitching_report_degrades_the_cached_groups_at_once() {
    let state = ekf_state();
    apply(&state, &[attitude(1_000), kinematics(1_000)]);
    apply(&state, &[ekf(HEALTHY)]);
    apply(&state, &[attitude(1_050), kinematics(1_050)]);
    apply(&state, &[ekf(HEALTHY | 32_768)]);

    let latest = state.lock().expect("lock");
    assert_eq!(
        latest.kinematics.expect("kinematics").quality,
        QUALITY_DEGRADED
    );
}

#[test]
fn other_dialects_ignore_the_report() {
    let state = Arc::new(Mutex::new(LinkState {
        authorization_source: crate::AuthorizationSource::StandardEstimatorStatus,
        ..LinkState::default()
    }));
    apply(&state, &[attitude(1_000)]);
    apply(&state, &[ekf(HEALTHY)]);
    assert!(state.lock().expect("lock").estimator_status.is_none());
}
