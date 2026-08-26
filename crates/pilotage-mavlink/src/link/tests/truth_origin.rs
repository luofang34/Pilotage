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
    assert!(
        update.pos_ned_m[1].is_finite(),
        "the east offset is a number: {:?}",
        update.pos_ned_m,
    );
}
