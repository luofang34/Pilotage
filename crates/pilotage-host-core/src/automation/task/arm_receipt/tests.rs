use pilotage_protocol::{CommandOutcome, wire};

use super::{ArmReceipt, ArmVerdict};

const TIMEOUT_NS: u64 = 3_000_000_000;

/// A flight-controller report: the heartbeat's arm state, and the
/// acknowledgement of the last arm, if one arrived.
fn report(armed: bool, ack: Option<CommandOutcome>) -> wire::FcState {
    wire::FcState {
        arm_state: if armed { 2 } else { 1 },
        stamp: Some(wire::MeasurementStamp::default()),
        last_command_kind: u32::from(ack.is_some()),
        last_command_outcome: ack.map_or(0, |outcome| outcome.to_wire() as i32),
        ..wire::FcState::default()
    }
}

#[test]
fn a_refused_first_arm_and_an_accepted_second_arm_give_their_own_receipts() {
    let mut receipt = ArmReceipt::default();
    // The first arm: the flight controller's preflight check refuses it.
    receipt.await_vehicle(7, 0, TIMEOUT_NS);
    assert_eq!(receipt.observe(&report(false, None)), None, "no answer yet");
    assert_eq!(
        receipt.observe(&report(false, Some(CommandOutcome::Denied))),
        Some((7, ArmVerdict::Refused(CommandOutcome::Denied)))
    );
    // The engine retries. The adapter clears the old acknowledgement
    // when it sends the new arm; the acknowledgement that accepts is the
    // answer, before the next heartbeat reports armed.
    receipt.await_vehicle(8, 1_000_000_000, TIMEOUT_NS);
    assert_eq!(
        receipt.observe(&report(false, Some(CommandOutcome::Accepted))),
        Some((8, ArmVerdict::Armed))
    );
    assert_eq!(
        receipt.observe(&report(true, None)),
        None,
        "one receipt per arm"
    );
}

#[test]
fn an_armed_heartbeat_without_an_acknowledgement_is_an_accepted_receipt() {
    let mut receipt = ArmReceipt::default();
    receipt.await_vehicle(3, 0, TIMEOUT_NS);
    assert_eq!(
        receipt.observe(&report(true, None)),
        Some((3, ArmVerdict::Armed))
    );
}

#[test]
fn an_unanswered_arm_is_refused_at_its_timeout() {
    let mut receipt = ArmReceipt::default();
    receipt.await_vehicle(4, 1_000, TIMEOUT_NS);
    assert_eq!(receipt.expire(1_000 + TIMEOUT_NS - 1), None);
    assert_eq!(
        receipt.expire(1_000 + TIMEOUT_NS),
        Some((4, ArmVerdict::Unanswered))
    );
    assert_eq!(receipt.expire(u64::MAX), None, "one receipt per arm");
}

#[test]
fn an_unstamped_arm_state_and_a_disarm_acknowledgement_answer_nothing() {
    let mut receipt = ArmReceipt::default();
    receipt.await_vehicle(5, 0, TIMEOUT_NS);
    let unstamped = wire::FcState {
        arm_state: 2,
        ..wire::FcState::default()
    };
    assert_eq!(receipt.observe(&unstamped), None);
    let disarm_refused = wire::FcState {
        last_command_kind: 2,
        last_command_outcome: CommandOutcome::Denied.to_wire() as i32,
        ..report(false, None)
    };
    assert_eq!(receipt.observe(&disarm_refused), None);
}
