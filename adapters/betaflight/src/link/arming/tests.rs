#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use super::{ARM_TIMEOUT, ArmSwitch, Requests};
use pilotage_adapter_api::FcCommandAck;
use pilotage_protocol::CommandOutcome;

fn req(arm: u32, disarm: u32) -> Requests {
    Requests {
        arm,
        disarm,
        refused: 0,
    }
}

#[test]
fn a_refusal_is_reported_without_moving_the_switch() {
    let mut switch = ArmSwitch::default();
    let refused = Requests {
        refused: 1,
        ..Requests::default()
    };
    assert!(!switch.update(refused, false, Instant::now()));
    assert_eq!(
        switch.last_command().map(|ack| ack.outcome),
        Some(CommandOutcome::TemporarilyRejected)
    );
}

#[test]
fn a_request_raises_the_switch_until_the_fc_arms() {
    let t0 = Instant::now();
    let mut switch = ArmSwitch::default();
    assert!(!switch.update(req(0, 0), false, t0));
    assert!(switch.update(req(1, 0), false, t0));
    assert!(switch.update(req(1, 0), false, t0 + Duration::from_millis(500)));
    assert!(switch.update(req(1, 0), true, t0 + Duration::from_millis(600)));
    assert_eq!(
        switch.last_command(),
        Some(FcCommandAck {
            arm: true,
            outcome: CommandOutcome::Accepted
        })
    );
}

#[test]
fn an_unanswered_request_drops_the_switch_and_reports_failure() {
    let t0 = Instant::now();
    let mut switch = ArmSwitch::default();
    assert!(switch.update(req(1, 0), false, t0));
    assert!(!switch.update(req(1, 0), false, t0 + ARM_TIMEOUT));
    assert_eq!(
        switch.last_command().map(|ack| ack.outcome),
        Some(CommandOutcome::Failed)
    );
    // A later blocker clearing does not arm: the request is spent.
    assert!(!switch.update(req(1, 0), false, t0 + ARM_TIMEOUT * 2));
}

#[test]
fn disarm_drops_the_switch_at_once_and_is_acknowledged_when_disarmed() {
    let t0 = Instant::now();
    let mut switch = ArmSwitch::default();
    switch.update(req(1, 0), false, t0);
    switch.update(req(1, 0), true, t0);
    assert!(!switch.update(req(1, 1), true, t0));
    assert!(switch.last_command().is_some_and(|ack| ack.arm));
    assert!(!switch.update(req(1, 1), false, t0));
    assert_eq!(
        switch.last_command(),
        Some(FcCommandAck {
            arm: false,
            outcome: CommandOutcome::Accepted
        })
    );
}

#[test]
fn the_switch_follows_an_fc_side_disarm() {
    let t0 = Instant::now();
    let mut switch = ArmSwitch::default();
    switch.update(req(1, 0), false, t0);
    assert!(switch.update(req(1, 0), true, t0));
    assert!(!switch.update(req(1, 0), false, t0));
    assert!(
        !switch.update(req(1, 0), true, t0),
        "no re-arm without a request"
    );
}
