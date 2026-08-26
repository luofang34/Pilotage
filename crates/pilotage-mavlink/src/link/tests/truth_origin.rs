//! The frame a simulator's truth reports are measured in.

#![allow(clippy::expect_used, clippy::panic)]

use std::time::Instant;

use crate::codec::FcMessage;
use crate::link::ResetPolicy;

use super::{SELECTED, apply_at, state};

/// The truth origin is latched once and holds for the life of the link, so
/// a report that states no position must not become it. Every later
/// position would then be measured from Null Island — a real place off the
/// coast of Africa — and the frame could never recover.
#[test]
fn a_truth_report_with_no_position_latches_no_origin() {
    let state = state(ResetPolicy::Conservative);
    let now = Instant::now();
    let truth = |lat_lon_alt| {
        (
            SELECTED,
            FcMessage::SimTruth {
                time_usec: 1_000,
                quat_wxyz: [1.0, 0.0, 0.0, 0.0],
                vel_ned_mps: [0.0; 3],
                lat_lon_alt,
            },
        )
    };

    apply_at(&state, &[truth([0, 0, 0])], now);
    {
        let latest = state.lock().expect("link state");
        assert!(
            latest.truth_origin.is_none(),
            "a report with no position latches no origin"
        );
        assert!(latest.sim_truth.is_none(), "and publishes no truth");
    }

    // A stated position latches, and the frame it defines is its own.
    apply_at(&state, &[truth([473_977_419, 85_455_938, 488_227])], now);
    let latest = state.lock().expect("link state");
    let origin = latest.truth_origin.expect("a stated position latches");
    assert_eq!(origin.lat_1e7, 473_977_419);
    let published = latest.sim_truth.expect("truth publishes");
    assert_eq!(published.lat_lon_alt, [473_977_419, 85_455_938, 488_227]);
    assert!(
        published.pos_ned_m.iter().all(|value| value.abs() < 1e-3),
        "the first stated position is the origin, so it sits at the origin"
    );
}

/// The whole lane an X-Plane session delivers, from the bytes on the wire.
///
/// The simulator's own satellite navigation reaches the flight controller,
/// and the controller forwards it in HIL_STATE_QUATERNION. Every value the
/// map draws from is read out of that payload at a fixed offset, so an
/// offset read one field early would publish a plausible position that is
/// not the one the simulator stated. The frame here is built and framed the
/// way the link receives one.
#[test]
fn a_real_truth_frame_carries_its_position_into_the_cache() {
    // Zurich, the datum the flight-deck world declares, at 488.227 m.
    const REPORTED: [i32; 3] = [473_977_419, 85_455_938, 488_227];

    let mut payload = vec![0_u8; 64];
    payload[0..8].copy_from_slice(&2_000_000_u64.to_le_bytes());
    payload[8..12].copy_from_slice(&1.0_f32.to_le_bytes());
    payload[36..40].copy_from_slice(&REPORTED[0].to_le_bytes());
    payload[40..44].copy_from_slice(&REPORTED[1].to_le_bytes());
    payload[44..48].copy_from_slice(&REPORTED[2].to_le_bytes());
    payload[48..50].copy_from_slice(&100_i16.to_le_bytes());
    let datagram =
        crate::codec::tests::encode_frame(crate::codec::HIL_STATE_QUATERNION_ID, &payload, true);

    let mut messages = Vec::new();
    let stats = crate::codec::parse_datagram(&datagram, &mut messages);
    assert_eq!(stats.crc_failures, 0, "the frame verifies");
    assert_eq!(messages.len(), 1, "the frame decodes: {messages:?}");

    let state = state(ResetPolicy::Conservative);
    crate::link::apply_messages_at(&state, &messages, 0, 0, Instant::now());

    let latest = state.lock().expect("link state");
    let truth = latest.sim_truth.expect("the forwarded truth report");
    assert_eq!(truth.lat_lon_alt, REPORTED);
}

/// The local frame is the stated position projected, so a report that
/// states no position states no frame either. Refusing only the geodetic
/// half leaves the projection: measured against a latched origin, an
/// all-zero triple reads thousands of kilometres from the vehicle, under a
/// flag that says the position is valid. A frame short of its geodetic
/// bytes decodes to the same zeros, and MAVLink 2 trims trailing zero bytes
/// on the wire, so this is a report a simulator can really send.
#[test]
fn a_report_with_no_position_is_refused_after_the_origin_is_latched() {
    const REPORTED: [i32; 3] = [473_977_419, 85_455_938, 488_227];
    let state = state(ResetPolicy::Conservative);
    let now = Instant::now();
    let truth = |lat_lon_alt| {
        (
            SELECTED,
            FcMessage::SimTruth {
                time_usec: 1_000,
                quat_wxyz: [1.0, 0.0, 0.0, 0.0],
                vel_ned_mps: [0.0; 3],
                lat_lon_alt,
            },
        )
    };
    apply_at(&state, &[truth(REPORTED)], now);
    apply_at(&state, &[truth([0, 0, 0])], now);

    let latest = state.lock().expect("link state");
    let update = latest.sim_truth.expect("the latched report");
    assert_eq!(
        update.lat_lon_alt, REPORTED,
        "the report that stated no position left no trace"
    );
    assert_eq!(
        update.pos_ned_m,
        [0.0, 0.0, 0.0],
        "the frame still measures the position the simulator did state"
    );
}

/// The origin is latched once and holds for the life of the link. A
/// simulator that restarted may have restarted into another world, and an
/// origin carried across the restart measures the new world's positions
/// from the old world's datum.
#[test]
fn a_new_source_epoch_releases_the_latched_origin() {
    const ZURICH: [i32; 3] = [473_977_419, 85_455_938, 488_227];
    let state = state(ResetPolicy::SimulatorHeuristic);
    let now = Instant::now();
    super::apply_at(&state, &[super::attitude_at(60_000, 0.5)], now);
    apply_at(
        &state,
        &[(
            SELECTED,
            FcMessage::SimTruth {
                time_usec: 1_000,
                quat_wxyz: [1.0, 0.0, 0.0, 0.0],
                vel_ned_mps: [0.0; 3],
                lat_lon_alt: ZURICH,
            },
        )],
        now,
    );
    assert!(state.lock().expect("link state").truth_origin.is_some());

    // The restart is detected the way the link really detects one: the
    // acquisition clock rewinds and the new stream holds for the dwell.
    super::confirm_attitude_reset(&state, now, 100);

    let latest = state.lock().expect("link state");
    assert_eq!(latest.source_epoch, 2, "the link saw the restart");
    assert!(
        latest.truth_origin.is_none(),
        "a restart releases the origin the old world latched"
    );
    assert!(latest.sim_truth.is_none());
}

/// The projection subtracts two wire values that each span the whole i32
/// range. Their difference does not fit one: a debug build panics and a
/// release build wraps into a frame measured from nowhere. Longitude is
/// where a real flight reaches the ends — a vehicle crossing the
/// antimeridian reports +179.9 and then -179.9 degrees.
#[test]
fn a_frame_across_the_antimeridian_does_not_overflow_the_projection() {
    let state = state(ResetPolicy::Conservative);
    let now = Instant::now();
    let truth = |lat_lon_alt| {
        (
            SELECTED,
            FcMessage::SimTruth {
                time_usec: 1_000,
                quat_wxyz: [1.0, 0.0, 0.0, 0.0],
                vel_ned_mps: [0.0; 3],
                lat_lon_alt,
            },
        )
    };
    apply_at(&state, &[truth([0, 1_799_000_000, 0])], now);
    apply_at(&state, &[truth([0, -1_799_000_000, 0])], now);

    let latest = state.lock().expect("link state");
    let update = latest.sim_truth.expect("the second report");
    // The value, not merely its finiteness: an f64 difference of two
    // i32-range values is always finite, so a finiteness check passes on
    // the wrapped garbage too. A release build has no overflow check, so
    // this assertion is the only thing standing between a wrapped
    // subtraction and a frame measured from nowhere.
    //
    // 359.8 degrees, not 0.2: the flat-earth projection subtracts raw
    // longitudes and does not wrap, so a frame that crosses the
    // antimeridian measures the long way round. That is a real property of
    // this projection and not the overflow — pinning the number states
    // both, where a finiteness check states neither.
    let expected_east = -359.8 * 111_111.0;
    assert!(
        (f64::from(update.pos_ned_m[1]) - expected_east).abs() < 10.0,
        "the east offset measures the real separation: {:?}",
        update.pos_ned_m,
    );
}

/// A receiver that states no position states no place. Zero on both angles
/// is a real place off the coast of Africa, so nothing downstream could
/// tell a receiver that has not solved from a vehicle that is there, and
/// the map would draw one.
#[test]
fn a_receiver_fix_at_zero_is_not_a_position() {
    let state = state(ResetPolicy::Conservative);
    let now = Instant::now();
    let fix = |lat_lon| {
        (
            SELECTED,
            FcMessage::GnssFix {
                time_usec: 2_000_000,
                lat_lon,
                alt_ellipsoid_mm: 536_000,
                accuracy_mm: [1_250, 2_100],
            },
        )
    };
    apply_at(&state, &[fix([0, 0])], now);
    assert!(
        state.lock().expect("link state").gnss_fix.is_none(),
        "a receiver that stated no position left no fix",
    );

    apply_at(&state, &[fix([473_977_419, 85_455_938])], now);
    let latest = state.lock().expect("link state");
    let update = latest.gnss_fix.expect("a stated position is a fix");
    assert_eq!(update.lat_lon, [473_977_419, 85_455_938]);
}

/// A receiver fix must not reach the shared boot-clock high water mark.
///
/// The estimate groups are ordered against one mark in boot milliseconds.
/// GPS_RAW_INT states a timestamp on a clock of the sender's choosing: a
/// flight controller with satellite time fills it from UTC, which in
/// milliseconds is a number decades past any boot clock. Fed into the
/// shared mark, it makes every later attitude and kinematics report look
/// like a restart, and the whole estimate stream stops.
#[test]
fn a_receiver_fix_does_not_disturb_the_estimate_groups() {
    // A UTC timestamp in microseconds, as a real receiver reports it.
    const UTC_USEC: u64 = 1_787_000_000_000_000;
    let state = state(ResetPolicy::SimulatorHeuristic);
    let now = Instant::now();

    super::apply_at(&state, &[super::attitude_at(5_000, 0.5)], now);
    let epoch_before = state.lock().expect("link state").source_epoch;

    apply_at(
        &state,
        &[(
            SELECTED,
            FcMessage::GnssFix {
                time_usec: UTC_USEC,
                lat_lon: [473_977_419, 85_455_938],
                alt_ellipsoid_mm: 536_000,
                accuracy_mm: [1_250, 2_100],
            },
        )],
        now,
    );
    assert!(
        state.lock().expect("link state").gnss_fix.is_some(),
        "the fix is still published",
    );

    // The estimate stream continues exactly as it would have.
    super::apply_at(&state, &[super::attitude_at(5_100, 0.6)], now);
    let latest = state.lock().expect("link state");
    let attitude = latest
        .attitude
        .expect("the next attitude report is accepted");
    assert_eq!(attitude.time_boot_ms, 5_100);
    assert_eq!(
        latest.source_epoch, epoch_before,
        "no restart was inferred from a clock the receiver chose",
    );
    assert_eq!(
        latest.reordered_measurements, 0,
        "no group was rejected because of a clock the receiver chose",
    );
    drop(latest);

    // A physical deployment admits no inter-group skew at all, and a
    // receiver reporting at 1 to 5 Hz always lags a 50 Hz attitude stream.
    // Ordered against the estimate groups its fix would never be accepted,
    // and a physical vehicle is the deployment this lane exists to serve.
    let physical = super::state(ResetPolicy::Conservative);
    physical
        .lock()
        .expect("link state")
        .maximum_inter_group_skew_ms = 0;
    super::apply_at(&physical, &[super::attitude_at(5_000, 0.5)], now);
    apply_at(
        &physical,
        &[(
            SELECTED,
            FcMessage::GnssFix {
                time_usec: 4_900_000,
                lat_lon: [473_977_419, 85_455_938],
                alt_ellipsoid_mm: 536_000,
                accuracy_mm: [1_250, 2_100],
            },
        )],
        now,
    );
    assert!(
        physical.lock().expect("link state").gnss_fix.is_some(),
        "a receiver that lags the attitude stream still states a position",
    );
}
