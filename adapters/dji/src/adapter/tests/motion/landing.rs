use super::*;
use pilotage_adapter_api::FlightProcedurePhase;
use pilotage_protocol::{ControlAction, ScopedControlFrame};

pub(super) fn land() -> ScopedControlFrame {
    let mut request = super::super::flight::request(ControlAction::Land, 72);
    let timing = request.admitted_timing.as_mut().unwrap();
    timing.active_remaining = None;
    timing.completion_timeout = Some(Duration::from_secs(30));
    request
}

fn flight_count(h: &Harness) -> usize {
    commands(h)
        .iter()
        .filter(|frame| frame.command_id() == 0x2a)
        .count()
}

#[test]
fn land_stops_sticks_and_waits_for_disable_ack_and_observation_before_one_command() {
    let mut h = enabled();
    assert_eq!(
        input(&mut h, 2, 20, [0.4, 0.0, 0.0, 0.0]),
        Disposition::Accepted
    );
    h.step(70);
    assert_eq!(h.apply(land(), 71).disposition, Disposition::Accepted);
    h.step(72);
    let last = commands(&h).pop().unwrap();
    assert_eq!(last.command_id(), 0x8e);
    assert!(
        last.payload()[1..]
            .as_chunks::<4>()
            .0
            .iter()
            .all(|v| f32::from_le_bytes(*v) == 0.0)
    );
    h.step(73);
    assert_eq!(commands(&h).last().unwrap().payload(), &[2]);
    ack(&mut h, VirtualStickMode::Disable, 3, 0);
    h.step(74);
    assert_eq!(flight_count(&h), 0);
    assert_eq!(
        input(&mut h, 3, 75, [0.4, 0.0, 0.0, 0.0]),
        Disposition::Accepted
    );
    assert_eq!(flight_count(&h), 0);
    assert_eq!(input(&mut h, 4, 76, [0.0; 4]), Disposition::Accepted);
    flying(&mut h, 3);
    h.step(200);
    flying(&mut h, 4);
    report(&mut h, 12, false, true, false);
    h.step(400);
    let sent = commands(&h);
    assert_eq!(sent.last().unwrap().command_id(), 0x2a);
    assert_eq!(sent.last().unwrap().payload(), &[2]);
    h.step(410);
    assert_eq!(flight_count(&h), 1);
    assert_eq!(input(&mut h, 5, 411, [0.0; 4]), Disposition::Accepted);
    h.step(420);
    assert_eq!(flight_count(&h), 1);
    assert_eq!(commands(&h).last().unwrap().command_id(), 0x2a);
}

#[test]
fn landing_never_renews_its_first_write_deadline_or_replays_after_cleanup_timeout() {
    for fail_before_write in [true, false] {
        let mut h = enabled();
        h.apply(land(), 20);
        if fail_before_write {
            h.set_write_limit(0).unwrap();
        }
        for n in 0..35_u16 {
            flying(&mut h, n + 3);
            h.step(21 + u64::from(n) * 100);
        }
        h.set_write_limit(4096).unwrap();
        ack(&mut h, VirtualStickMode::Disable, 3, 0);
        report(&mut h, 12, false, true, false);
        flying(&mut h, 40);
        h.step(3600);
        assert_eq!(flight_count(&h), 0);
        let phase = h
            .adapter
            .sample_telemetry()
            .samples
            .into_iter()
            .find_map(|sample| sample.flight_procedure)
            .unwrap()
            .phase;
        assert!(matches!(
            phase,
            FlightProcedurePhase::TimedOut | FlightProcedurePhase::Cancelled
        ));
    }
}

#[test]
fn authority_loss_cancels_a_landing_waiting_for_mode_release() {
    let mut h = enabled();
    h.apply(land(), 20);
    h.step(21);
    h.step(22);
    h.adapter.invalidate_scope(
        pilotage_protocol::VehicleId::new(1),
        &ScopeId::new(FLIGHT_SCOPE),
        Duration::from_millis(23),
    );
    ack(&mut h, VirtualStickMode::Disable, 3, 0);
    report(&mut h, 12, false, true, false);
    flying(&mut h, 3);
    h.step(24);
    assert_eq!(flight_count(&h), 0);
}
