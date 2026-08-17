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
