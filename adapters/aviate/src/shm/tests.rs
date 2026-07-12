#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use super::{SHM_SIZE, ShmFreshness, ShmObservation, decode_sample, enu_quat_to_ned, enu_to_ned};

fn put_f64(buf: &mut [u8; SHM_SIZE], off: usize, v: f64) {
    buf[off..off + 8].copy_from_slice(&v.to_ne_bytes());
}

#[test]
fn enu_to_ned_matches_aviates_test_vector() {
    // 1 m east, 2 m north, 3 m up → 2 m north, 1 m east, 3 m down
    // (the test vector in Aviate's plugin.rs).
    assert_eq!(enu_to_ned([1.0, 2.0, 3.0]), [2.0, 1.0, -3.0]);
}

#[test]
fn identity_enu_flu_attitude_is_heading_east_in_ned() {
    // FLU body aligned with ENU world: body forward = +x_ENU = east,
    // so the NED/FRD yaw must be +90°.
    let q = enu_quat_to_ned([1.0, 0.0, 0.0, 0.0]);
    let (w, x, y, z) = (
        f64::from(q[0]),
        f64::from(q[1]),
        f64::from(q[2]),
        f64::from(q[3]),
    );
    let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    assert!(
        (yaw - core::f64::consts::FRAC_PI_2).abs() < 1e-5,
        "yaw {yaw}"
    );
}

#[test]
fn block_decodes_positions_velocities_and_time() {
    let mut buf = [0u8; SHM_SIZE];
    // pos ENU (east 1, north 2, up 3).
    put_f64(&mut buf, super::OFF_POS, 1.0);
    put_f64(&mut buf, super::OFF_POS + 8, 2.0);
    put_f64(&mut buf, super::OFF_POS + 16, 3.0);
    // identity quaternion.
    put_f64(&mut buf, super::OFF_QUAT, 1.0);
    // vel ENU (0.5 east, 0 north, -1 up = descending).
    put_f64(&mut buf, super::OFF_VEL, 0.5);
    put_f64(&mut buf, super::OFF_VEL + 16, -1.0);
    buf[super::OFF_TIME_US..super::OFF_TIME_US + 8].copy_from_slice(&42_000_000u64.to_ne_bytes());

    let s = decode_sample(&buf, 7);
    assert_eq!(s.pos_ned_m, [2.0, 1.0, -3.0]);
    assert_eq!(s.vel_ned_mps, [0.0, 0.5, 1.0]);
    assert_eq!(s.time_us, 42_000_000);
    assert_eq!(s.seq, 7);
}

#[test]
fn frozen_sample_never_revives_without_a_new_identity() {
    let start = Instant::now();
    let mut freshness = ShmFreshness::new_at(start);
    assert_eq!(
        freshness.observe_at(7, 42_000, start),
        ShmObservation::Advancing
    );
    assert_eq!(
        freshness.observe_at(7, 42_000, start + Duration::from_secs(4)),
        ShmObservation::Unchanged(Duration::from_secs(4))
    );
    assert_eq!(
        freshness.observe_at(7, 42_000, start + Duration::from_secs(8)),
        ShmObservation::Unchanged(Duration::from_secs(8))
    );
}

#[test]
fn same_object_rollback_is_quarantined_but_sequence_wrap_is_valid() {
    let start = Instant::now();
    let mut wrapped = ShmFreshness::new_at(start);
    assert_eq!(
        wrapped.observe_at(u32::MAX, 100, start),
        ShmObservation::Advancing
    );
    assert_eq!(
        wrapped.observe_at(0, 101, start + Duration::from_millis(1)),
        ShmObservation::Advancing
    );

    let mut reset = ShmFreshness::new_at(start);
    assert_eq!(reset.observe_at(100, 100, start), ShmObservation::Advancing);
    assert_eq!(
        reset.observe_at(1, 1, start + Duration::from_millis(1)),
        ShmObservation::Quarantined
    );
    assert_eq!(
        reset.observe_at(101, 101, start + Duration::from_secs(1)),
        ShmObservation::Quarantined
    );

    let mut unchanged_clock = ShmFreshness::new_at(start);
    assert_eq!(
        unchanged_clock.observe_at(10, 500, start),
        ShmObservation::Advancing
    );
    assert_eq!(
        unchanged_clock.observe_at(11, 500, start + Duration::from_millis(1)),
        ShmObservation::Quarantined
    );
}
