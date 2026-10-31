//! A fixed-wing disarm ends the stick stream on the FC's word.

use std::time::Duration;

use pilotage_mavlink::codec::encode_command_ack;

use super::{SYSTEM, Sent, flying, sticks_of, taking_off};

#[test]
fn an_accepted_disarm_stops_the_stream() {
    let mut harness = taking_off();
    harness.uplink.send_disarm();
    assert!(harness.sent().contains(&Sent::Arm(false)));
    let accepted = encode_command_ack(0, 400, 0, SYSTEM, 1);
    harness.deliver(&[&accepted], Some(0.0));
    harness.sent();
    harness.tick(Duration::from_millis(200), Some(0.0));
    let sent = harness.sent();
    assert!(sticks_of(&sent).is_empty(), "{sent:?}");
}

#[test]
fn a_refused_disarm_keeps_the_sticks_flowing_until_the_fc_reports_disarmed() {
    let mut harness = flying();
    harness.uplink.send_disarm();
    let refused = encode_command_ack(0, 400, 1, SYSTEM, 1);
    harness.deliver(&[&refused], Some(0.0));
    harness.sent();
    harness.tick(Duration::from_millis(200), Some(0.0));
    let sent = harness.sent();
    assert!(!sticks_of(&sent).is_empty(), "{sent:?}");

    harness.report_mode(false, 0, Some(0.0));
    harness.sent();
    harness.tick(Duration::from_millis(200), Some(0.0));
    let sent = harness.sent();
    assert!(sticks_of(&sent).is_empty(), "{sent:?}");
}
