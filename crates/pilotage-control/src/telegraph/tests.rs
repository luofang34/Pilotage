//! The telegraph's safety contract: orders send once, answers come only
//! from the FC's own report, and every involuntary exit from an ordered
//! state snaps the lever back to SAFE — never a hidden retry.

#![allow(clippy::expect_used, clippy::panic)]

use super::{ArmConfirmed, ArmOrder, ArmTelegraph, TelegraphPhase};

#[test]
fn an_order_sends_once_and_settles_on_the_fc_report() {
    let mut telegraph = ArmTelegraph::default();
    telegraph.on_fc_arm_state(1);

    let action = telegraph.set_order(ArmOrder::Armed).expect("order sends");
    assert_eq!(action.action, 1);
    assert_eq!(*telegraph.phase(), TelegraphPhase::AwaitingAnswer);

    // An accepted verdict is not an answer; the lamp waits for the FC.
    telegraph.on_action_result(1, true, "");
    assert_eq!(*telegraph.phase(), TelegraphPhase::AwaitingAnswer);
    assert_eq!(telegraph.confirmed(), ArmConfirmed::Disarmed);

    telegraph.on_fc_arm_state(2);
    assert_eq!(*telegraph.phase(), TelegraphPhase::InSync);
    assert_eq!(telegraph.confirmed(), ArmConfirmed::Armed);

    // Re-ordering the answered state commands nothing.
    assert!(telegraph.set_order(ArmOrder::Armed).is_none());
}

#[test]
fn a_refusal_snaps_the_lever_back_with_the_reason() {
    let mut telegraph = ArmTelegraph::default();
    telegraph.on_fc_arm_state(1);
    telegraph.set_order(ArmOrder::Armed);
    telegraph.on_action_result(1, false, "sender does not hold the scope");
    assert_eq!(telegraph.order(), ArmOrder::Safe);
    assert_eq!(
        *telegraph.phase(),
        TelegraphPhase::Refused("sender does not hold the scope".to_owned())
    );
}

#[test]
fn a_unilateral_disarm_drops_the_order_and_never_rearms() {
    let mut telegraph = ArmTelegraph::default();
    telegraph.on_fc_arm_state(1);
    telegraph.set_order(ArmOrder::Armed);
    telegraph.on_fc_arm_state(2);
    assert_eq!(*telegraph.phase(), TelegraphPhase::InSync);

    // The failsafe (or the ground auto-disarm) takes the vehicle back.
    telegraph.on_fc_arm_state(1);
    assert_eq!(telegraph.order(), ArmOrder::Safe, "the lever snaps back");
    assert_eq!(*telegraph.phase(), TelegraphPhase::Dropped);

    // Nothing is pending: only a fresh human order arms again.
    assert!(telegraph.set_order(ArmOrder::Safe).is_none());
}

#[test]
fn a_safe_order_disarms_and_settles() {
    let mut telegraph = ArmTelegraph::default();
    telegraph.on_fc_arm_state(2);
    let action = telegraph.set_order(ArmOrder::Safe).expect("disarm sends");
    assert_eq!(action.action, 2);
    telegraph.on_fc_arm_state(1);
    assert_eq!(*telegraph.phase(), TelegraphPhase::InSync);
    assert_eq!(telegraph.confirmed(), ArmConfirmed::Disarmed);
}

#[test]
fn the_fcs_own_refusal_answers_the_order_with_its_reason() {
    let mut telegraph = ArmTelegraph::default();
    telegraph.on_fc_arm_state(1);
    telegraph.set_order(ArmOrder::Armed);
    // The host passes the arm on; the FC refuses it (preflight).
    telegraph.on_action_result(1, true, "");
    telegraph.on_fc_command_verdict(1, 1);
    assert_eq!(telegraph.order(), ArmOrder::Safe);
    assert!(
        matches!(telegraph.phase(), TelegraphPhase::Refused(why) if why.contains("not ready")),
        "{:?}",
        telegraph.phase()
    );
}

#[test]
fn a_verdict_standing_before_the_order_does_not_answer_it_at_once() {
    let mut telegraph = ArmTelegraph::default();
    telegraph.on_fc_arm_state(1);
    telegraph.on_fc_command_verdict(1, 1);
    telegraph.set_order(ArmOrder::Armed);
    // Every telemetry sample repeats the standing verdict.
    telegraph.on_fc_command_verdict(1, 1);
    assert_eq!(*telegraph.phase(), TelegraphPhase::AwaitingAnswer);
    // Unchanged past the window, the repeated refusal is the answer.
    telegraph.on_tick(0.0);
    telegraph.on_tick(super::ANSWER_WINDOW_MS);
    assert!(matches!(telegraph.phase(), TelegraphPhase::Refused(why) if why.contains("not ready")));
}

#[test]
fn an_order_nothing_answers_snaps_back_and_says_so() {
    let mut telegraph = ArmTelegraph::default();
    telegraph.on_fc_arm_state(1);
    telegraph.set_order(ArmOrder::Armed);
    telegraph.on_tick(100.0);
    telegraph.on_tick(100.0 + super::ANSWER_WINDOW_MS - 1.0);
    assert_eq!(*telegraph.phase(), TelegraphPhase::AwaitingAnswer);
    telegraph.on_tick(100.0 + super::ANSWER_WINDOW_MS);
    assert_eq!(telegraph.order(), ArmOrder::Safe);
    assert_eq!(
        *telegraph.phase(),
        TelegraphPhase::Refused("no answer from the vehicle".to_owned())
    );
}

#[test]
fn an_accepted_or_in_progress_verdict_leaves_the_answer_to_the_lamp() {
    let mut telegraph = ArmTelegraph::default();
    telegraph.on_fc_arm_state(1);
    telegraph.set_order(ArmOrder::Armed);
    telegraph.on_fc_command_verdict(1, 5);
    telegraph.on_fc_command_verdict(1, 0);
    assert_eq!(*telegraph.phase(), TelegraphPhase::AwaitingAnswer);
    telegraph.on_fc_arm_state(2);
    assert_eq!(*telegraph.phase(), TelegraphPhase::InSync);
}

#[test]
fn an_arm_carried_out_after_the_window_still_answers_its_order() {
    let mut telegraph = ArmTelegraph::default();
    telegraph.on_fc_arm_state(1);
    telegraph.set_order(ArmOrder::Armed);
    telegraph.on_tick(0.0);
    telegraph.on_tick(super::ANSWER_WINDOW_MS);
    assert_eq!(telegraph.order(), ArmOrder::Safe);
    // A starved link delivers the arm late: the lever must not read SAFE
    // over a live vehicle, and nothing is re-sent.
    telegraph.on_fc_arm_state(2);
    assert_eq!(telegraph.order(), ArmOrder::Armed);
    assert_eq!(*telegraph.phase(), TelegraphPhase::InSync);
    assert_eq!(telegraph.confirmed(), ArmConfirmed::Armed);
}

#[test]
fn a_late_answer_is_forgotten_once_a_new_order_is_given() {
    let mut telegraph = ArmTelegraph::default();
    telegraph.on_fc_arm_state(1);
    telegraph.set_order(ArmOrder::Armed);
    telegraph.on_tick(0.0);
    telegraph.on_tick(super::ANSWER_WINDOW_MS);
    assert!(telegraph.set_order(ArmOrder::Safe).is_none());
    telegraph.on_fc_arm_state(2);
    assert_eq!(telegraph.order(), ArmOrder::Safe);
}

#[test]
fn arming_a_vehicle_left_armed_takes_it_back_under_command() {
    // A fresh session finds the vehicle armed: a ground station dropped
    // and came back while a failsafe flies it.
    let mut telegraph = ArmTelegraph::default();
    telegraph.on_fc_arm_state(2);
    assert_eq!(telegraph.order(), ArmOrder::Safe);
    let action = telegraph
        .set_order(ArmOrder::Armed)
        .expect("the arm is sent");
    assert_eq!(action.action, 1);
    assert_eq!(*telegraph.phase(), TelegraphPhase::AwaitingAnswer);
    // The vehicle's next report, still armed, answers it.
    telegraph.on_fc_arm_state(2);
    assert_eq!(*telegraph.phase(), TelegraphPhase::InSync);
    assert!(telegraph.set_order(ArmOrder::Armed).is_none());
}
