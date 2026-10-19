#![allow(clippy::expect_used)]

use std::time::{Duration, Instant};

use super::ArmTracker;
use crate::LinkState;

#[test]
fn only_a_new_heartbeat_advances_the_arm_report() {
    let mut latest = LinkState::default();
    let mut tracker = ArmTracker::default();
    tracker.observe(&latest);
    assert!(tracker.report().is_none(), "no heartbeat, no report");

    let first = Instant::now();
    latest.last_heartbeat = Some(first);
    latest.heartbeat_armed = Some(false);
    tracker.observe(&latest);
    tracker.observe(&latest);
    let report = tracker.report().expect("a report");
    assert_eq!((report.armed, report.sequence), (false, 0));

    latest.last_heartbeat = Some(first + Duration::from_secs(1));
    latest.heartbeat_armed = Some(true);
    tracker.observe(&latest);
    let report = tracker.report().expect("a report");
    assert_eq!((report.armed, report.sequence), (true, 1));
}
