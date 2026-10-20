#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use pilotage_timing::MonoTimestamp;

use super::PoseToFrame;

const SIMULATION: i32 = 2;

fn at(ms: u64) -> MonoTimestamp {
    MonoTimestamp::from_nanos(ms * 1_000_000)
}

#[test]
fn a_frame_is_timed_from_the_truth_sample_it_was_rendered_from() {
    let mut tracker = PoseToFrame::default();
    tracker.note_truth(SIMULATION, 4_000, at(100));
    // A re-broadcast of the same sample keeps its first receipt.
    tracker.note_truth(SIMULATION, 4_000, at(110));
    tracker.note_truth(SIMULATION, 8_000, at(104));
    tracker.note_frame(0, 2, 4_000, at(125));
    tracker.note_frame(2, 2, 8_000, at(144));
    let (p50, _, _, count) = tracker.summary(0).expect("fpv measured");
    assert_eq!((p50, count), (Duration::from_millis(25), 1));
    let (p50, _, _, _) = tracker.summary(2).expect("gimbal measured");
    assert_eq!(p50, Duration::from_millis(40));
    assert!(tracker.summary(1).is_none(), "no chase frames");
}

#[test]
fn frames_without_their_truth_sample_are_not_counted() {
    let mut tracker = PoseToFrame::default();
    tracker.note_truth(SIMULATION, 4_000, at(100));
    // Another clock, another time, an unknown source, and a frame that
    // arrived before its truth sample are all skipped.
    tracker.note_frame(0, 3, 4_000, at(120));
    tracker.note_frame(0, 2, 5_000, at(120));
    tracker.note_frame(7, 2, 4_000, at(120));
    tracker.note_frame(0, 2, 4_000, at(90));
    assert!(tracker.summary(0).is_none());
}

#[test]
fn the_truth_window_is_bounded() {
    let mut tracker = PoseToFrame::default();
    for sample in 0..5_000_u64 {
        tracker.note_truth(SIMULATION, sample, at(sample));
    }
    tracker.note_frame(0, 2, 0, at(9_000));
    assert!(
        tracker.summary(0).is_none(),
        "the oldest samples are forgotten"
    );
    tracker.note_frame(0, 2, 4_999, at(5_000));
    assert!(tracker.summary(0).is_some());
}
