#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use super::{
    Action, Event, INIT_SETTLE, INIT_TIMEOUT, MAX_LAUNCHES, MAX_PROVISIONS, MOTOR_PACKETS_TO_FLY,
    MOTOR_TIMEOUT, Supervisor,
};

fn started(provision: bool, t0: Instant) -> Supervisor {
    let mut supervisor = Supervisor::new(provision);
    supervisor.handle(Event::Start, t0);
    supervisor
}

#[test]
fn a_healthy_start_provisions_launches_streams_and_flies() {
    let t0 = Instant::now();
    let mut s = Supervisor::new(true);
    assert_eq!(s.handle(Event::Start, t0), vec![Action::Provision]);
    assert_eq!(
        s.handle(Event::ProvisionFinished { ok: true }, t0),
        vec![Action::Launch]
    );
    assert!(
        s.handle(Event::MotorPackets(500), t0).is_empty(),
        "not streaming yet"
    );
    assert!(s.handle(Event::InitComplete, t0).is_empty(), "settling");
    assert!(s.handle(Event::Tick, t0 + INIT_SETTLE / 2).is_empty());
    assert_eq!(
        s.handle(Event::Tick, t0 + INIT_SETTLE),
        vec![Action::StartStreaming]
    );
    assert!(
        s.handle(Event::MotorPackets(MOTOR_PACKETS_TO_FLY - 1), t0)
            .is_empty()
    );
    assert_eq!(
        s.handle(Event::MotorPackets(1), t0),
        vec![Action::ReportFlying]
    );
    assert!(s.is_flying());
}

#[test]
fn a_failed_provision_retries_up_to_its_cap_then_gives_up() {
    let t0 = Instant::now();
    let mut s = started(true, t0);
    for _ in 1..MAX_PROVISIONS {
        assert_eq!(
            s.handle(Event::ProvisionFinished { ok: false }, t0),
            vec![Action::Provision]
        );
    }
    assert!(matches!(
        s.handle(Event::ProvisionFinished { ok: false }, t0)[..],
        [Action::GiveUp { .. }]
    ));
    assert!(
        s.handle(Event::Start, t0).is_empty(),
        "a given-up supervisor stays down"
    );
}

#[test]
fn a_crash_before_init_relaunches_without_streaming() {
    let t0 = Instant::now();
    let mut s = started(false, t0);
    assert_eq!(
        s.handle(Event::Exited, t0),
        vec![Action::StopStreaming, Action::Launch]
    );
    s.handle(Event::InitComplete, t0);
    assert_eq!(
        s.handle(Event::Tick, t0 + INIT_SETTLE),
        vec![Action::StartStreaming]
    );
}

#[test]
fn an_init_that_hangs_is_killed_and_relaunched() {
    let t0 = Instant::now();
    let mut s = started(false, t0);
    assert!(s.handle(Event::Tick, t0 + INIT_TIMEOUT / 2).is_empty());
    assert!(matches!(
        s.handle(Event::Tick, t0 + INIT_TIMEOUT)[..],
        [Action::StopStreaming, Action::Kill { .. }]
    ));
    assert_eq!(
        s.handle(Event::Exited, t0 + INIT_TIMEOUT),
        vec![Action::StopStreaming, Action::Launch]
    );
}

#[test]
fn a_start_whose_pid_loop_never_runs_is_killed_and_relaunched() {
    let t0 = Instant::now();
    let mut s = started(false, t0);
    s.handle(Event::InitComplete, t0);
    s.handle(Event::Tick, t0);
    let streaming = t0 + INIT_SETTLE;
    s.handle(Event::Tick, streaming);
    s.handle(Event::MotorPackets(1), streaming);
    assert!(
        s.handle(Event::Tick, streaming + MOTOR_TIMEOUT / 2)
            .is_empty()
    );
    let actions = s.handle(Event::Tick, streaming + MOTOR_TIMEOUT);
    assert!(
        matches!(actions[..], [Action::StopStreaming, Action::Kill { reason }] if reason.contains("PID"))
    );
    assert_eq!(
        s.handle(Event::Exited, streaming + MOTOR_TIMEOUT),
        vec![Action::StopStreaming, Action::Launch]
    );
}

#[test]
fn launches_are_bounded_between_healthy_starts() {
    let t0 = Instant::now();
    let mut s = started(false, t0);
    for _ in 1..MAX_LAUNCHES {
        assert_eq!(
            s.handle(Event::Exited, t0),
            vec![Action::StopStreaming, Action::Launch]
        );
    }
    assert!(matches!(
        s.handle(Event::Exited, t0)[..],
        [Action::StopStreaming, Action::GiveUp { .. }]
    ));
}

#[test]
fn a_crash_in_flight_relaunches_with_a_fresh_budget() {
    let t0 = Instant::now();
    let mut s = started(false, t0);
    for _ in 1..MAX_LAUNCHES {
        s.handle(Event::Exited, t0);
    }
    s.handle(Event::InitComplete, t0);
    s.handle(Event::Tick, t0 + INIT_SETTLE);
    s.handle(Event::MotorPackets(MOTOR_PACKETS_TO_FLY), t0 + INIT_SETTLE);
    assert!(s.is_flying());
    let later = t0 + Duration::from_secs(60);
    assert_eq!(
        s.handle(Event::Exited, later),
        vec![Action::StopStreaming, Action::Launch]
    );
    assert!(!s.is_flying());
}
