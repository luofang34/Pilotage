#![allow(clippy::expect_used)]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::{GateEvent, OwnerEvent, StartedTarget, receive_target_start};

#[test]
fn parent_close_requests_containment_before_target_start() {
    let (sender, events) = mpsc::channel();
    let (containment, contained) = mpsc::channel();
    sender
        .send(OwnerEvent::ParentClosed)
        .expect("queue parent closure");
    let gate = std::thread::Builder::new()
        .name("aviate-parent-close-test-gate".to_owned())
        .spawn(move || {
            contained.recv().expect("observe containment request");
            sender
                .send(OwnerEvent::Gate(Ok(GateEvent::TargetStarted { pid: 41 })))
                .expect("queue target identity");
        })
        .expect("spawn test gate");
    let mut gate_failed = false;
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(1))
        .expect("make target-start deadline");

    let started = receive_target_start(&events, deadline, &mut gate_failed, || {
        containment.send(()).expect("request target containment");
    })
    .expect("retain target identity after parent closure");
    gate.join().expect("test gate completes");

    assert_eq!(
        started,
        StartedTarget {
            pid: 41,
            parent_closed: true,
        }
    );
    assert!(!gate_failed);
}
