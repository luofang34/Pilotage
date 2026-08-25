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
