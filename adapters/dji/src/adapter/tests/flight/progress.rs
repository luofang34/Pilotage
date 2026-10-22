use std::time::Duration;

use pilotage_adapter_api::{
    CommandOutcome, Disposition, FlightProcedureKind, FlightProcedurePhase as Phase,
    FlightProcedureReason as Reason,
};
use pilotage_protocol::ControlAction;

use super::*;

#[test]
fn disconnect_retains_accepted_unknown_result_and_cannot_replay_commands() {
    let mut h = ready();
    let frame = request(ControlAction::Takeoff, 77);
    h.apply(frame.clone(), 20);
    h.step(30);
    let sequence = commands(&h)[0].sequence();
    feed(&mut h, &ack(sequence, 0));
    h.step(40);
    assert_eq!(progress(&mut h).phase, Phase::InProgress);
    let written = h.written().unwrap();
    h.adapter.disconnect_blocking();
    assert!(h.adapter.transport_released());
    let unknown = progress(&mut h);
    assert_eq!(unknown.phase, Phase::OutcomeUnknown);
    assert_eq!(unknown.fc_outcome, Some(CommandOutcome::Accepted));
    assert_eq!(unknown.request.action_id, 77);
    assert!(matches!(
        h.apply(frame, 50).disposition,
        Disposition::Rejected(_)
    ));
    h.step(60);
    h.adapter.disconnect_blocking();
    assert_eq!(progress(&mut h), unknown);
    assert_eq!(h.written().unwrap(), written);
}

#[test]
fn exact_request_written_ack_and_motor_reports_never_claim_completed() {
    let mut h = ready();
    let frame = request(ControlAction::Takeoff, 77);
    assert_eq!(
        h.apply(frame.clone(), 20).disposition,
        Disposition::Accepted
    );
    let queued = progress(&mut h);
    assert_eq!(queued.phase, Phase::Queued);
    assert_eq!(queued.request.session, frame.session);
    assert_eq!(queued.request.generation, frame.generation);
    assert_eq!(queued.request.scope, frame.scope);
    assert_eq!(queued.request.sequence, frame.sequence);
    assert_eq!(queued.request.action_id, 77);
    h.step(30);
    assert_eq!(progress(&mut h).phase, Phase::Written);
    let sequence = commands(&h)[0].sequence();
    feed(&mut h, &ack(sequence, 0));
    feed_fc(&mut h, 2, 8);
    h.step(40);
    let accepted = progress(&mut h);
    assert_eq!(accepted.phase, Phase::InProgress);
    assert_eq!(accepted.fc_outcome, Some(CommandOutcome::Accepted));
    feed(&mut h, &ack(sequence, 74));
    feed_fc(&mut h, 3, 12);
    h.step(50);
    assert_eq!(progress(&mut h), accepted);
    feed_fc(&mut h, 4, 0);
    h.step(60);
    assert_eq!(progress(&mut h), accepted);
    assert_eq!(commands(&h).len(), 1);
    assert!(matches!(
        h.apply(request(ControlAction::Takeoff, 78), 60).disposition,
        Disposition::Rejected(_)
    ));
}

#[test]
fn only_matching_postwrite_ack_is_accepted_and_negative_ack_is_sticky() {
    let mut h = ready();
    h.apply(request(ControlAction::Takeoff, 1), 20);
    feed(&mut h, &ack(2, 0));
    h.step(30);
    assert_eq!(progress(&mut h).phase, Phase::Written);
    let sequence = commands(&h)[0].sequence();
    for case in 0..5 {
        let mut bytes = ack(sequence, 0);
        match case {
            0 => bytes[4] = 14,
            1 => bytes[5] = 10,
            2 => bytes[8] = 0,
            3 => bytes[10] = 0x2b,
            _ => bytes[6] = bytes[6].wrapping_add(1),
        }
        seal_duml(&mut bytes);
        feed(&mut h, &bytes);
        h.step(40 + case);
        assert_eq!(progress(&mut h).phase, Phase::Written);
    }
    feed(&mut h, &ack(sequence, 74));
    h.step(50);
    let failed = progress(&mut h);
    assert_eq!(failed.phase, Phase::Failed);
    assert_eq!(failed.reason, Some(Reason::CommandRefused));
    feed(&mut h, &ack(sequence, 0));
    h.step(60);
    assert_eq!(progress(&mut h), failed);
    // No raw result-code whitelist establishes that a written command was inert.
    assert!(matches!(
        h.apply(request(ControlAction::Takeoff, 2), 60).disposition,
        Disposition::Rejected(_)
    ));
}

#[test]
fn missing_or_late_ack_and_completion_timeout_do_not_retry() {
    for accepted in [false, true] {
        let mut h = ready();
        let mut frame = request(ControlAction::Takeoff, 1);
        frame.admitted_timing.as_mut().unwrap().completion_timeout =
            Some(Duration::from_millis(150));
        h.apply(frame, 20);
        h.step(30);
        let sequence = commands(&h)[0].sequence();
        if accepted {
            feed(&mut h, &ack(sequence, 0));
            h.step(40);
        }
        h.step(170);
        let timed_out = progress(&mut h);
        assert_eq!(timed_out.phase, Phase::TimedOut);
        assert_eq!(timed_out.reason, Some(Reason::DeadlineExpired));
        feed(&mut h, &ack(sequence, 0));
        h.step(180);
        assert_eq!(progress(&mut h), timed_out);
        assert!(matches!(
            h.apply(request(ControlAction::Takeoff, 2), 180).disposition,
            Disposition::Rejected(_)
        ));
        assert_eq!(commands(&h).len(), 1);
    }
}

#[test]
fn explicit_land_replaces_takeoff_tracking_without_automatic_cancellation_command() {
    let mut h = ready();
    h.apply(request(ControlAction::Takeoff, 1), 20);
    h.step(30);
    let takeoff_sequence = commands(&h)[0].sequence();
    feed_fc(&mut h, 2, 8);
    h.step(40);
    assert_eq!(
        h.apply(request(ControlAction::Land, 2), 40).disposition,
        Disposition::Accepted
    );
    h.step(50);
    let packets = commands(&h);
    assert_eq!(packets.len(), 2);
    assert_eq!(packets[1].payload(), &[2]);
    feed(&mut h, &ack(takeoff_sequence, 0));
    h.step(60);
    assert_eq!(progress(&mut h).kind, FlightProcedureKind::Land);
    assert_eq!(progress(&mut h).phase, Phase::Written);
    feed(&mut h, &ack(packets[1].sequence(), 0));
    feed_fc(&mut h, 3, 0);
    h.step(70);
    assert_eq!(progress(&mut h).phase, Phase::InProgress);
    assert_eq!(commands(&h).len(), 2);
}

#[test]
fn flight_completion_requires_ack_and_independent_forward_reports() {
    for (action, complete_flags) in [(ControlAction::Takeoff, 12), (ControlAction::Land, 0)] {
        let mut h = ready();
        h.apply(request(action, 1), 20);
        h.step(30);
        let sequence = commands(&h)[0].sequence();
        feed(&mut h, &ack(sequence, 0));
        feed_fc(&mut h, 2, complete_flags);
        feed_fc(&mut h, 3, complete_flags);
        feed_fc(&mut h, 4, complete_flags);
        h.step(40);
        assert_eq!(progress(&mut h).phase, Phase::InProgress);
        h.step(50);
        feed_fc(&mut h, 3, complete_flags);
        h.step(60);
        assert_eq!(progress(&mut h).phase, Phase::InProgress);
        feed_fc(&mut h, 5, complete_flags);
        h.step(70);
        assert_eq!(progress(&mut h).phase, Phase::InProgress);
        feed_fc(&mut h, 6, complete_flags);
        h.step(80);
        let completed = progress(&mut h);
        assert_eq!(completed.phase, Phase::Completed);
        assert_eq!(completed.fc_outcome, Some(CommandOutcome::Accepted));
        feed_fc(&mut h, 7, 0);
        h.step(90);
        assert_eq!(progress(&mut h), completed);
        assert_eq!(commands(&h).len(), 1);
    }
}
