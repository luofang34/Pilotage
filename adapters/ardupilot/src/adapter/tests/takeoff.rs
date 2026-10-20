use pilotage_adapter_api::{Disposition, FlightProcedurePhase, VehicleAdapter};
use pilotage_mavlink::LandedState;
use pilotage_mavlink::codec::{FcMessage, MAV_CMD_NAV_TAKEOFF};
use pilotage_protocol::{ActionKind, ControlAction, ScopeId, VehicleId};

use super::{Harness, frame, scoped, velocity};
use crate::adapter::{DIRECT_SCOPE, FLIGHT_SCOPE, GIMBAL_SCOPE};

fn ready(harness: &mut Harness) {
    harness.estimate(0.0, true);
    harness.feed(&[
        FcMessage::LandedState(LandedState::OnGround),
        FcMessage::RelativeAltitude {
            time_boot_ms: harness.time_boot_ms,
            altitude_mm: 5_000,
        },
    ]);
}

#[test]
fn takeoff_is_advertised_only_on_the_velocity_flight_scope() {
    let harness = Harness::new();
    let capabilities = harness.adapter.capabilities();
    for scope in &capabilities.vehicles[0].scopes {
        let supported = scope
            .actions
            .iter()
            .any(|action| action.action == ActionKind::Takeoff);
        assert_eq!(supported, scope.scope.as_str() == FLIGHT_SCOPE);
        for action in &scope.actions {
            assert_eq!(
                action.takeoff_start_state,
                (action.action == ActionKind::Takeoff)
                    .then_some(pilotage_protocol::TakeoffStartState::ArmedOnGround)
            );
        }
    }
}

#[test]
fn explicit_takeoff_queues_home_relative_target_and_preserves_request_identity() {
    let mut harness = Harness::new();
    ready(&mut harness);
    let request = frame(None, vec![ControlAction::Takeoff]);
    let outcome = harness.adapter.apply_control(&request);
    assert_eq!(outcome.disposition, Disposition::Accepted);
    assert!(outcome.action_results[0].accepted);
    let command = harness.commands.try_recv().expect("queued command");
    assert_eq!(command.command, MAV_CMD_NAV_TAKEOFF);
    assert_eq!(command.params[6], 7.5, "5 m above home plus 2.5 m climb");
    assert!(command.fence.as_ref().unwrap().is_valid());
    assert!(command.send_before.is_some());
    assert!(command.receipt.as_ref().unwrap().sent_at_cached().is_none());
    let before = harness.adapter.sample_telemetry().samples[0]
        .flight_procedure
        .clone()
        .expect("procedure status");
    let after = harness.adapter.sample_telemetry().samples[0]
        .flight_procedure
        .clone()
        .expect("retained status");
    assert_eq!(before, after, "polling does not renew a transition");
    assert_eq!(before.phase, FlightProcedurePhase::Queued);
    assert_eq!(before.request.session, request.session);
    assert_eq!(before.request.action_id, 1);
    assert_eq!(before.request.scope.as_str(), FLIGHT_SCOPE);
    assert!(harness.velocity.borrow().is_none());
}

#[test]
fn an_unobserved_ground_state_refuses_takeoff_without_writes() {
    let mut harness = Harness::new();
    harness.estimate(0.0, true);
    let outcome = harness
        .adapter
        .apply_control(&frame(None, vec![ControlAction::Takeoff]));
    assert!(matches!(outcome.disposition, Disposition::Rejected(_)));
    assert!(!outcome.action_results[0].accepted);
    assert!(harness.commands.try_recv().is_err());
}

#[test]
fn conflicting_or_wrong_scope_takeoff_never_arms_or_writes() {
    let mut harness = Harness::new();
    ready(&mut harness);
    for request in [
        frame(None, vec![ControlAction::Arm, ControlAction::Takeoff]),
        frame(None, vec![ControlAction::Disarm, ControlAction::Takeoff]),
        frame(None, vec![ControlAction::Takeoff, ControlAction::Takeoff]),
        scoped(DIRECT_SCOPE, None, vec![ControlAction::Takeoff]),
        frame(velocity(1.0, -1.0, 0.0), vec![ControlAction::Takeoff]),
    ] {
        assert!(matches!(
            harness.adapter.apply_control(&request).disposition,
            Disposition::Rejected(_)
        ));
    }
    assert!(harness.commands.try_recv().is_err());
}

#[test]
fn flight_revocation_fences_queued_takeoff_but_payload_revocation_does_not() {
    let mut harness = Harness::new();
    ready(&mut harness);
    let result = harness
        .adapter
        .apply_control(&frame(None, vec![ControlAction::Takeoff]));
    assert_eq!(result.disposition, Disposition::Accepted);
    let command = harness.commands.try_recv().expect("queued takeoff");
    let fence = command.fence.expect("command fence");
    for (vehicle, scope) in [
        (VehicleId::new(2), FLIGHT_SCOPE),
        (VehicleId::new(1), GIMBAL_SCOPE),
    ] {
        harness
            .adapter
            .invalidate_pending_commands(vehicle, &ScopeId::new(scope));
        assert!(fence.is_valid());
    }
    harness
        .adapter
        .invalidate_pending_commands(VehicleId::new(1), &ScopeId::new(FLIGHT_SCOPE));
    assert!(!fence.is_valid());
    let status = harness.adapter.sample_telemetry().samples[0]
        .flight_procedure
        .clone()
        .expect("cancelled procedure");
    assert_eq!(status.phase, FlightProcedurePhase::Cancelled);
}

#[test]
fn refused_action_queue_is_not_reported_as_accepted() {
    let mut harness = Harness::new();
    harness.commands.close();
    for action in [ControlAction::Arm, ControlAction::Disarm] {
        let outcome = harness.adapter.apply_control(&frame(None, vec![action]));
        assert!(matches!(outcome.disposition, Disposition::Rejected(_)));
    }
}
