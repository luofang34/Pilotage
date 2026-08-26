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
