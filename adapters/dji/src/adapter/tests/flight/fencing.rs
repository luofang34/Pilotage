use std::time::Duration;

use pilotage_adapter_api::{
    Disposition, FlightProcedurePhase as Phase, FlightProcedureReason as Reason, VehicleAdapter,
};
use pilotage_protocol::{ControlAction, Generation, ScopeId, VehicleId};

use super::*;

#[test]
fn queued_write_rechecks_expiry_model_and_motor_state() {
    for case in 0..5 {
        let mut h = ready();
        h.apply(request(ControlAction::Takeoff, 1), 20);
        let before = h.written().unwrap();
        match case {
            0 => h.step(120),
            1 => {
                feed_fc(&mut h, 2, 8);
                h.step(30);
            }
            2 => {
                feed_battery(&mut h, 2, 24);
                h.step(30);
            }
            3 => {
                h.feed_model(86, 2).unwrap();
                h.feed_model(87, 3).unwrap();
                h.step(30);
            }
            _ => {
                h.feed_model(87, 2).unwrap();
                h.step(400);
            }
        }
        assert_eq!(h.written().unwrap(), before);
        assert!(matches!(
            progress(&mut h).phase,
            Phase::TimedOut | Phase::Cancelled
        ));
    }
}

#[test]
fn unrelated_scope_invalidation_does_not_cancel_flight_but_authority_loss_does() {
    let mut h = ready();
    h.apply(request(ControlAction::Takeoff, 1), 20);
    h.adapter
        .invalidate_pending_commands(VehicleId::new(2), &ScopeId::new(FLIGHT_SCOPE));
    h.adapter
        .invalidate_pending_commands(VehicleId::new(1), &ScopeId::new("vehicle.gimbal"));
    assert_eq!(progress(&mut h).phase, Phase::Queued);
    h.adapter.invalidate_scope(
        VehicleId::new(1),
        &ScopeId::new(FLIGHT_SCOPE),
        Duration::from_millis(25),
    );
    h.step(30);
    let status = progress(&mut h);
    assert_eq!(status.phase, Phase::Cancelled);
    assert_eq!(status.reason, Some(Reason::AuthorityLost));
    assert!(commands(&h).is_empty());
    assert!(matches!(
        h.apply(request(ControlAction::Takeoff, 2), 30).disposition,
        Disposition::Rejected(_)
    ));
    let mut newer = request(ControlAction::Takeoff, 3);
    newer.generation = Generation::new(2);
    assert_eq!(h.apply(newer, 30).disposition, Disposition::Accepted);
    h.step(40);
    assert_eq!(commands(&h).len(), 1);
}

#[test]
fn partial_write_invalidations_close_transport_without_splicing_land() {
    for source_change in [false, true] {
        let mut h = ready();
        h.set_write_limit(3).unwrap();
        h.apply(request(ControlAction::Takeoff, 1), 20);
        h.step(30);
        let before = h.written().unwrap();
        if source_change {
            h.feed_model(86, 2).unwrap();
            h.step(40);
        } else {
            assert!(matches!(
                h.apply(request(ControlAction::Land, 2), 40).disposition,
                Disposition::Rejected(_)
            ));
        }
        assert!(h.adapter.status_cached().fault.is_some());
        h.set_write_limit(usize::MAX).unwrap();
        h.step(50);
        assert_eq!(h.written().unwrap(), before);
    }
}

#[test]
fn lost_write_receipt_or_capture_failure_never_reports_accepted_or_retries() {
    for capture_failure in [false, true] {
        let mut h = ready();
        h.apply(request(ControlAction::Takeoff, 1), 20);
        if capture_failure {
            h.fail_capture().unwrap();
        } else {
            h.lose_write_receipt().unwrap();
        }
        h.step(30);
        let status = progress(&mut h);
        assert_eq!(status.phase, Phase::OutcomeUnknown);
        assert_eq!(status.fc_outcome, None);
        assert_eq!(status.reason, Some(Reason::WriteFailed));
        let before = h.written().unwrap();
        feed(&mut h, &ack(2, 0));
        h.step(40);
        assert_eq!(h.written().unwrap(), before);
        assert_eq!(progress(&mut h), status);
    }
}

#[test]
fn a_late_full_write_is_not_promoted_by_a_late_ack() {
    let mut h = ready();
    h.apply(request(ControlAction::Takeoff, 1), 20);
    let mut times = [30, 30, 121, 121, 121].into_iter();
    h.adapter
        .step_with_time_blocking(|| Duration::from_millis(times.next().unwrap()));
    assert_eq!(progress(&mut h).phase, Phase::TimedOut);
    let sequence = commands(&h)[0].sequence();
    feed(&mut h, &ack(sequence, 0));
    h.step(122);
    assert_eq!(progress(&mut h).phase, Phase::TimedOut);
    assert_eq!(commands(&h).len(), 1);
}

#[test]
fn accepted_ack_cannot_rebind_a_procedure_across_source_change() {
    let mut h = ready();
    h.apply(request(ControlAction::Takeoff, 1), 20);
    h.step(30);
    let sequence = commands(&h)[0].sequence();
    feed(&mut h, &ack(sequence, 0));
    h.feed_model(86, 2).unwrap();
    h.feed_model(87, 3).unwrap();
    h.step(40);
    let cancelled = progress(&mut h);
    assert_eq!(cancelled.phase, Phase::Cancelled);
    assert_eq!(cancelled.reason, Some(Reason::SourceChanged));
    assert_eq!(
        cancelled.fc_outcome,
        Some(pilotage_adapter_api::CommandOutcome::Accepted)
    );
    feed_fc(&mut h, 4, 0);
    feed_battery(&mut h, 2, 50);
    h.step(50);
    assert_eq!(progress(&mut h), cancelled);
    assert!(matches!(
        h.apply(request(ControlAction::Takeoff, 2), 50).disposition,
        Disposition::Rejected(_)
    ));
    assert_eq!(
        h.apply(request(ControlAction::Land, 3), 50).disposition,
        Disposition::Accepted
    );
}

#[test]
fn short_flight_writes_keep_the_original_deadline_and_wait_for_the_complete_packet() {
    let mut h = ready();
    h.apply(request(ControlAction::Takeoff, 1), 20);
    h.set_write_limit(4).unwrap();
    h.step(30);
    assert_eq!(progress(&mut h).phase, Phase::Queued);
    let first_deadline = *h.deadlines().unwrap().last().unwrap();
    h.set_write_limit(usize::MAX).unwrap();
    h.step(40);
    assert_eq!(progress(&mut h).phase, Phase::Written);
    assert_eq!(*h.deadlines().unwrap().last().unwrap(), first_deadline);
    assert_eq!(commands(&h).len(), 1);
    assert_eq!(h.recorded("tx").unwrap(), h.written().unwrap());
}

#[test]
fn flight_preempts_waiting_keyframe_but_never_splices_a_partial_keyframe() {
    for partial in [false, true] {
        let mut h = ready();
        let video = h.adapter.subscribe_encoded_video().unwrap();
        h.set_write_limit(if partial { 4 } else { 0 }).unwrap();
        video
            .keyframe_request
            .store(true, std::sync::atomic::Ordering::Release);
        h.step(20);
        let before = h.written().unwrap();
        let result = h.apply(request(ControlAction::Land, 1), 30);
        h.set_write_limit(usize::MAX).unwrap();
        h.step(40);
        if partial {
            assert!(matches!(result.disposition, Disposition::Rejected(_)));
            assert!(h.adapter.status_cached().fault.is_some());
            assert_eq!(h.written().unwrap(), before);
        } else {
            assert_eq!(result.disposition, Disposition::Accepted);
            assert_eq!(commands(&h).len(), 1);
        }
    }
}

#[test]
fn shorter_completion_budget_also_fences_the_first_write() {
    let mut h = ready();
    let mut frame = request(ControlAction::Takeoff, 1);
    frame.admitted_timing.as_mut().unwrap().completion_timeout = Some(Duration::from_millis(5));
    assert_eq!(h.apply(frame, 20).disposition, Disposition::Accepted);
    h.step(25);
    assert!(commands(&h).is_empty());
    assert_eq!(progress(&mut h).phase, Phase::TimedOut);
    assert_eq!(progress(&mut h).reason, Some(Reason::DeadlineExpired));
    assert_eq!(
        h.apply(request(ControlAction::Takeoff, 2), 30).disposition,
        Disposition::Accepted
    );
}
