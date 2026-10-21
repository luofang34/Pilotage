use super::*;
use pilotage_adapter_api::{FlightProcedurePhase, FlightProcedureReason};

fn landing() -> Harness {
    let mut h = enabled();
    assert_eq!(
        h.apply(landing::land(), 20).disposition,
        Disposition::Accepted
    );
    h.step(21);
    h.step(22);
    ack(&mut h, VirtualStickMode::Disable, 3, 0);
    report(&mut h, 12, false, true, false);
    flying_mode(&mut h, 3, 12);
    h.step(23);
    let packet = commands(&h).pop().unwrap();
    assert_eq!(packet.command_id(), 0x2a);
    assert_eq!(packet.payload(), [2]);
    let response = super::super::flight::ack(packet.sequence(), 0);
    super::super::flight::feed(&mut h, &response);
    h.step(24);
    h
}

fn cancel(h: &mut Harness) -> u16 {
    assert_eq!(input(h, 2, 25, [0.4, 0.0, 0.0, 0.0]), Disposition::Accepted);
    h.step(25);
    assert_eq!(
        commands(h).last().unwrap().payload(),
        [2],
        "held input must not cancel landing"
    );
    input(h, 3, 26, [0.0; 4]);
    input(h, 4, 27, [0.4, 0.0, 0.0, 0.0]);
    h.step(28);
    let packet = commands(h).pop().unwrap();
    assert_eq!(packet.command_id(), 0x2a);
    assert_eq!(packet.payload(), [14]);
    packet.sequence()
}

#[test]
fn fresh_manual_takeover_cancels_landing_then_requires_ack_hover_and_enabled_feedback() {
    let mut h = landing();
    let sequence = cancel(&mut h);
    flying_mode(&mut h, 4, 6);
    let response = super::super::flight::ack(sequence, 0);
    super::super::flight::feed(&mut h, &response);
    h.step(29);
    input(&mut h, 5, 29, [0.4, 0.0, 0.0, 0.0]);
    h.step(30);
    assert_eq!(commands(&h).last().unwrap().payload(), [14]);
    flying_mode(&mut h, 5, 6);
    h.step(31);
    let progress = h
        .adapter
        .sample_telemetry()
        .samples
        .into_iter()
        .find_map(|s| s.flight_procedure)
        .unwrap();
    assert_eq!(progress.phase, FlightProcedurePhase::Cancelled);
    assert_eq!(progress.reason, Some(FlightProcedureReason::Interrupted));
    assert_eq!(
        input(&mut h, 6, 32, [0.4, 0.0, 0.0, 0.0]),
        Disposition::Accepted
    );
    h.step(33);
    let enable = commands(&h).pop().unwrap();
    assert_eq!(enable.command_id(), 0x80);
    assert_eq!(enable.payload(), [1]);
    ack(&mut h, VirtualStickMode::Enable, enable.sequence(), 0);
    report(&mut h, 13, true, true, false);
    h.step(34);
    input(&mut h, 7, 35, [0.4, 0.0, 0.0, 0.0]);
    h.step(90);
    let packet = commands(&h).pop().unwrap();
    assert_eq!(packet.command_id(), 0x8e);
    assert_eq!(
        f32::from_le_bytes(packet.payload()[1..5].try_into().unwrap()),
        0.4
    );
    assert_eq!(
        commands(&h)
            .iter()
            .filter(|p| p.command_id() == 0x2a)
            .count(),
        2
    );
    h.step(150);
    h.step(151);
    assert!(matches!(
        input(&mut h, 8, 152, [0.4, 0.0, 0.0, 0.0]),
        Disposition::Rejected(_)
    ));
}

#[test]
fn missing_or_rejected_cancel_response_cannot_resume_manual_velocity_or_retry_cancel() {
    for rejected in [false, true] {
        let mut h = landing();
        let sequence = cancel(&mut h);
        if rejected {
            super::super::flight::feed(&mut h, &super::super::flight::ack(sequence, 147));
        }
        for tick in 0..25_u16 {
            flying_mode(&mut h, tick + 4, 6);
            let now = 40 + u64::from(tick) * 100;
            h.step(now);
            input(&mut h, u32::from(tick) + 5, now, [0.4, 0.0, 0.0, 0.0]);
        }
        super::super::flight::feed(&mut h, &super::super::flight::ack(sequence, 0));
        h.step(2500);
        assert_eq!(commands(&h).last().unwrap().payload(), [14]);
        assert_eq!(
            commands(&h)
                .iter()
                .filter(|p| p.command_id() == 0x2a)
                .count(),
            2
        );
        for (sequence, at) in [(30, 2510), (31, 2520), (32, 2530)] {
            super::super::flight::feed_fc(&mut h, sequence, 0);
            h.step(at);
        }
        let progress = h
            .adapter
            .sample_telemetry()
            .samples
            .into_iter()
            .find_map(|sample| sample.flight_procedure)
            .unwrap();
        assert_eq!(progress.phase, FlightProcedurePhase::Completed);
    }
}

#[test]
fn landing_without_an_existing_stick_session_accepts_only_a_fresh_neutral_takeover() {
    let mut h = ready();
    h.adapter.experimental_virtual_stick = true;
    flying_mode(&mut h, 2, 6);
    h.step(11);
    h.apply(landing::land(), 12);
    h.step(13);
    let land = commands(&h).pop().unwrap();
    super::super::flight::feed(&mut h, &super::super::flight::ack(land.sequence(), 0));
    flying_mode(&mut h, 3, 12);
    h.step(14);
    assert_eq!(input(&mut h, 1, 15, [0.0; 4]), Disposition::Accepted);
    for repeated in [1, 0] {
        assert!(matches!(
            input(&mut h, repeated, 16, [0.4, 0.0, 0.0, 0.0]),
            Disposition::Rejected(_)
        ));
    }
    h.step(17);
    assert_eq!(commands(&h).last().unwrap().payload(), [2]);
    assert_eq!(
        input(&mut h, 2, 18, [0.4, 0.0, 0.0, 0.0]),
        Disposition::Accepted
    );
    h.step(19);
    assert_eq!(commands(&h).last().unwrap().payload(), [14]);
}
