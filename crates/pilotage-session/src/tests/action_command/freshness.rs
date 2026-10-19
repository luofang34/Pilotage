use super::*;
use pilotage_protocol::{CommandTiming, Ping};
use std::time::Duration;

fn issue_window(engine: &mut SessionEngine, client: ClientKey, now: u64) -> u64 {
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Ping(Ping {
            nonce: 1,
            sender_sent_at: MonoTimestamp::from_nanos(0),
        }),
        MonoTimestamp::from_nanos(now),
    );
    outcome
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: OutboundMessage::Pong(pong),
                ..
            } => Some(pong.control_window_id),
            _ => None,
        })
        .expect("issued window")
}

fn timed_command(session: SessionId, generation: Generation, window: u64) -> DomainEnvelope {
    let mut message = command(session, generation, ControlAction::Arm, 10);
    if let DomainEnvelope::ActionCommand(value) = &mut message {
        value.sampled_at = MonoTimestamp::from_nanos(9_000_000_000);
        value.timing = Some(CommandTiming {
            require_control_window: window != 0,
            control_window_id: window,
            ..CommandTiming::default()
        });
    }
    message
}

#[test]
fn reliable_action_queue_time_consumes_the_first_message_budget() {
    let (mut engine, client, session, generation) = bound_engine();
    let outcome = engine.handle_received_message(
        client,
        timed_command(session, generation, 0),
        MonoTimestamp::from_nanos(10_000_000),
        MonoTimestamp::from_nanos(260_000_000),
    );
    assert!(rejected_detail(&outcome).contains("timing expired"));
}

#[test]
fn adapter_receives_only_the_unspent_window_and_queue_budget() {
    let (mut engine, client, session, generation) = bound_engine();
    let window = issue_window(&mut engine, client, 10_000_000);
    let outcome = engine.handle_received_message(
        client,
        timed_command(session, generation, window),
        MonoTimestamp::from_nanos(30_000_000),
        MonoTimestamp::from_nanos(60_000_000),
    );
    let timing = outcome
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::ApplyToAdapter { frame, .. } => frame.admitted_timing,
            _ => None,
        })
        .expect("admitted timing");
    assert_eq!(timing.send_remaining, Duration::from_millis(200));
    assert_eq!(timing.active_remaining, Some(Duration::from_millis(200)));
    assert_eq!(timing.admitted_at.as_nanos(), 60_000_000);
    assert!(timing.window_bounded);
}

#[test]
fn fresh_receive_stamp_cannot_renew_an_expired_host_window() {
    let (mut engine, client, session, generation) = bound_engine();
    let window = issue_window(&mut engine, client, 10_000_000);
    let outcome = engine.handle_received_message(
        client,
        timed_command(session, generation, window),
        MonoTimestamp::from_nanos(260_000_000),
        MonoTimestamp::from_nanos(260_000_000),
    );
    assert!(rejected_detail(&outcome).contains("control window unavailable"));
}

#[test]
fn a_window_from_another_client_does_not_grant_validity() {
    let (mut engine, client, session, generation) = bound_engine();
    let other = ClientKey::new(2);
    welcome(&mut engine, other);
    let window = issue_window(&mut engine, other, 10_000_000);
    let outcome = engine.handle_received_message(
        client,
        timed_command(session, generation, window),
        MonoTimestamp::from_nanos(20_000_000),
        MonoTimestamp::from_nanos(20_000_000),
    );
    assert!(rejected_detail(&outcome).contains("control window unavailable"));
}

#[test]
fn a_new_window_cannot_renew_input_sampled_before_its_probe() {
    let (mut engine, client, session, generation) = bound_engine();
    let response = engine.handle_client_message(
        client,
        DomainEnvelope::Ping(Ping {
            nonce: 2,
            sender_sent_at: MonoTimestamp::from_nanos(10_000_000_000),
        }),
        MonoTimestamp::from_nanos(10_000_000),
    );
    let window = response
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: OutboundMessage::Pong(pong),
                ..
            } => Some(pong.control_window_id),
            _ => None,
        })
        .unwrap();
    let result = engine.handle_received_message(
        client,
        timed_command(session, generation, window),
        MonoTimestamp::from_nanos(20_000_000),
        MonoTimestamp::from_nanos(20_000_000),
    );
    assert!(rejected_detail(&result).contains("control window unavailable"));
}

#[test]
fn completion_deadline_does_not_restart_after_admission_queueing() {
    let (mut engine, client, session, generation) = bound_engine();
    let window = issue_window(&mut engine, client, 10_000_000);
    let mut message = timed_command(session, generation, window);
    if let DomainEnvelope::ActionCommand(value) = &mut message {
        value.timing.as_mut().unwrap().completion_timeout = Some(Duration::from_secs(1));
    }
    let outcome = engine.handle_received_message(
        client,
        message,
        MonoTimestamp::from_nanos(30_000_000),
        MonoTimestamp::from_nanos(60_000_000),
    );
    let timing = outcome
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::ApplyToAdapter { frame, .. } => frame.admitted_timing,
            _ => None,
        })
        .unwrap();
    assert_eq!(timing.completion_timeout, Some(Duration::from_millis(950)));
}
