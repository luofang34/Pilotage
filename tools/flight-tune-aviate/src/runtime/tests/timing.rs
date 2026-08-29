//! The sample clock and the phase windows measured on it.

use crate::runtime::timing::{PhaseClock, SampleClock};

use super::stamp;

#[test]
fn the_clock_accepts_frames_in_order_and_counts_them() {
    let mut clock = SampleClock::new();
    assert_eq!(clock.accepted(), 0);
    assert_eq!(clock.last_source_sequence(), None);
    for index in 1..=4_u64 {
        clock
            .accept(stamp(index, index * 12_500_000))
            .expect("accept an advancing frame");
    }
    assert_eq!(clock.accepted(), 4);
    assert_eq!(clock.last_source_sequence(), Some(4));
}

#[test]
fn a_repeated_or_reordered_frame_is_refused() {
    let mut clock = SampleClock::new();
    clock.accept(stamp(4, 40)).expect("accept the first frame");
    let detail = clock
        .accept(stamp(4, 50))
        .expect_err("a repeated sequence must fail")
        .to_string();
    assert!(detail.contains("does not advance"), "{detail}");
    let detail = clock
        .accept(stamp(3, 30))
        .expect_err("an earlier sequence must fail")
        .to_string();
    assert!(detail.contains("does not advance"), "{detail}");
    assert_eq!(clock.accepted(), 1, "a refused frame is not counted");
}

#[test]
fn a_frame_time_that_steps_backward_is_refused() {
    let mut clock = SampleClock::new();
    clock.accept(stamp(1, 100)).expect("accept the first frame");
    let detail = clock
        .accept(stamp(2, 50))
        .expect_err("a backward time must fail")
        .to_string();
    assert!(detail.contains("steps backward"), "{detail}");
}

#[test]
fn a_trial_time_after_the_simulator_time_is_refused() {
    let mut clock = SampleClock::new();
    let detail = clock
        .accept(crate::runtime::timing::FrameStamp {
            source_sequence: 1,
            simulator_time_ns: 10,
            trial_time_ns: 20,
        })
        .expect_err("a trial time after the simulator time must fail")
        .to_string();
    assert!(detail.contains("after the simulator time"), "{detail}");
}

#[test]
fn a_phase_measures_from_the_frame_that_opened_it() {
    let mut phase = PhaseClock::new();
    assert!(!phase.is_entered());
    phase.enter(stamp(1, 1_000));
    // A second directive on a later frame does not restart a window that
    // is already running.
    phase.enter(stamp(2, 5_000));
    assert_eq!(
        phase.entered().map(|entry| entry.trial_time_ns),
        Some(1_000)
    );
    assert_eq!(phase.elapsed_ns(stamp(3, 4_000)).expect("elapsed"), 3_000);
    phase.leave();
    assert!(!phase.is_entered());
    phase
        .elapsed_ns(stamp(4, 6_000))
        .expect_err("a closed phase has no window");
}
