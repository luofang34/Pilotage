use pilotage_adapter_api::{Disposition, VehicleAdapter};
use pilotage_protocol::{
    ActionKind, ControlAction, ControlIntent, GimbalRateIntent, TakeoffStartState,
};

use super::*;

#[test]
fn only_opted_in_upper_observed_attachment_advertises_discrete_motion() {
    for mut h in [
        Harness::new(),
        Harness::mobile(),
        Harness::flight(DjiControlMode::Observed, Framing::Duml),
        Harness::flight(DjiControlMode::BenchInitialHorizontal, Framing::LogicLink),
    ] {
        assert!(
            h.adapter.capabilities().vehicles[0]
                .scopes
                .iter()
                .all(|scope| scope.scope.as_str() != FLIGHT_SCOPE)
        );
        assert!(matches!(
            h.apply(request(ControlAction::Takeoff, 1), 10).disposition,
            Disposition::Rejected(_)
        ));
    }
    let h = ready();
    let capability = h.adapter.capabilities();
    let scope = capability.vehicles[0]
        .scopes
        .iter()
        .find(|scope| scope.scope.as_str() == FLIGHT_SCOPE)
        .unwrap();
    assert!(scope.axes.is_empty() && scope.intents.is_empty() && scope.legacy.is_none());
    assert!(capability.vehicles[0].is_command_only_group(&scope.scope));
    assert_eq!(
        scope
            .actions
            .iter()
            .map(|action| action.action)
            .collect::<Vec<_>>(),
        [ActionKind::Takeoff, ActionKind::Land]
    );
    assert_eq!(
        scope.actions[0].takeoff_start_state,
        Some(TakeoffStartState::MotorsStopped)
    );
}

#[test]
fn running_flying_low_battery_and_unverified_probe_never_admit_takeoff() {
    for (flags, percent) in [(8, 50), (12, 50), (0, 24)] {
        let mut h = ready();
        feed_fc(&mut h, 2, flags);
        feed_battery(&mut h, 2, percent);
        h.step(20);
        assert!(matches!(
            h.apply(request(ControlAction::Takeoff, 1), 20).disposition,
            Disposition::Rejected(_)
        ));
        h.step(30);
        assert!(commands(&h).is_empty());
    }
    let mut h = Harness::flight(DjiControlMode::Observed, Framing::LogicLink);
    feed_fc(&mut h, 1, 0);
    feed_battery(&mut h, 1, 50);
    h.step(10);
    assert!(matches!(
        h.apply(request(ControlAction::Takeoff, 1), 10).disposition,
        Disposition::Rejected(_)
    ));
}

#[test]
fn land_accepts_running_motors_and_low_or_absent_battery() {
    let mut h = Harness::flight(DjiControlMode::Observed, Framing::LogicLink);
    feed(&mut h, &probe());
    feed_fc(&mut h, 1, 12);
    remote(&mut h, 1, [1024; 4]);
    h.step(10);
    assert_eq!(
        h.apply(request(ControlAction::Land, 1), 10).disposition,
        Disposition::Accepted
    );
    h.step(20);
    assert_eq!(commands(&h)[0].payload(), &[2]);
    let mut h = ready();
    feed_fc(&mut h, 2, 8);
    feed_battery(&mut h, 2, 1);
    h.step(20);
    assert_eq!(
        h.apply(request(ControlAction::Land, 1), 20).disposition,
        Disposition::Accepted
    );
}

#[test]
fn discrete_scope_refuses_axes_legacy_missing_identity_and_expired_window() {
    let mut h = ready();
    for case in 0..6 {
        let mut frame = request(ControlAction::Takeoff, case + 1);
        match case {
            0 => {
                frame.intent = Some(ControlIntent::GimbalRate(GimbalRateIntent {
                    pitch_rate: 0.0,
                    yaw_rate: 0.0,
                }))
            }
            1 => frame.actions.push(ControlAction::Land),
            2 => frame.action_ids.clear(),
            3 => frame.admitted_timing = None,
            4 => frame.admitted_timing.as_mut().unwrap().window_bounded = false,
            _ => frame.admitted_timing.as_mut().unwrap().send_remaining = std::time::Duration::ZERO,
        }
        assert!(matches!(
            h.apply(frame, 20).disposition,
            Disposition::Rejected(_)
        ));
    }
    h.step(30);
    assert!(commands(&h).is_empty());
}

#[test]
fn battery_must_be_observed_under_the_current_model_binding_and_cannot_renew_by_replay() {
    let mut h = Harness::flight(DjiControlMode::Observed, Framing::LogicLink);
    feed(&mut h, &probe());
    remote(&mut h, 1, [1024; 4]);
    feed_battery(&mut h, 1, 50);
    feed_fc(&mut h, 1, 0);
    h.step(10);
    assert!(matches!(
        h.apply(request(ControlAction::Takeoff, 1), 10).disposition,
        Disposition::Rejected(_)
    ));
    feed_battery(&mut h, 1, 50);
    h.step(20);
    assert!(matches!(
        h.apply(request(ControlAction::Takeoff, 2), 20).disposition,
        Disposition::Rejected(_)
    ));
    feed_battery(&mut h, 2, 50);
    h.step(30);
    assert_eq!(
        h.apply(request(ControlAction::Takeoff, 3), 30).disposition,
        Disposition::Accepted
    );
}

#[test]
fn fresh_fc_reports_do_not_renew_battery_age_and_land_remains_available() {
    let mut h = ready();
    for sequence in 2..=13 {
        feed_fc(&mut h, sequence, 0);
        remote(&mut h, sequence, [1024; 4]);
        feed_battery(&mut h, 1, 50);
        h.step(10 + u64::from(sequence - 1) * 250);
    }
    assert!(matches!(
        h.apply(request(ControlAction::Takeoff, 1), 3_010)
            .disposition,
        Disposition::Rejected(_)
    ));
    assert_eq!(
        h.apply(request(ControlAction::Land, 2), 3_010).disposition,
        Disposition::Accepted
    );
    h.step(3_020);
    assert_eq!(commands(&h)[0].payload(), &[2]);
}
