//! The reliable action-command path (CTRL-01): every command is bound to
//! session, vehicle, scope, fencing generation, and announced activation
//! revision, and every failure is answered with an explicit rejected
//! result — a delayed or replayed press can never fire under authority it
//! no longer holds.

use pilotage_protocol::{
    ControlAction, ControlActionCommand, Generation, LeaseRelease, LeaseRequest, SessionId,
};
use pilotage_timing::MonoTimestamp;

use super::{VEHICLE, engine, motion, welcome};
use crate::{ClientKey, DomainEnvelope, OutboundMessage, SessionAction, SessionEngine};

fn announce(engine: &mut SessionEngine, client: ClientKey, session: SessionId, revision: u32) {
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::ProfileActivation(pilotage_protocol::ProfileActivation {
            session,
            profile_id: "builtin.flight.default".to_owned(),
            profile_revision: 1,
            activation_revision: revision,
            digest: [0x11; 32],
            device_profile_id: String::new(),
            device_profile_revision: 0,
            device_digest: [0; 32],
        }),
        MonoTimestamp::from_nanos(1),
    );
    assert!(
        matches!(
            outcome.actions.as_slice(),
            [SessionAction::ActivationAccepted { .. }]
        ),
        "the engine emits its explicit acceptance event: {:?}",
        outcome.actions
    );
}

fn grant_motion(engine: &mut SessionEngine, client: ClientKey) -> Generation {
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Lease(LeaseRequest {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        MonoTimestamp::from_nanos(2),
    );
    outcome
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: OutboundMessage::LeaseResponse(response),
                ..
            } if response.granted => Some(response.generation),
            _ => None,
        })
        .expect("granted")
}

fn command(
    session: SessionId,
    generation: Generation,
    action: ControlAction,
    action_id: u32,
) -> DomainEnvelope {
    DomainEnvelope::ActionCommand(ControlActionCommand {
        session,
        vehicle: VEHICLE,
        scope: motion(),
        generation,
        activation_revision: 1,
        action,
        action_id,
    })
}

/// The single rejected result an outcome carries, panicking otherwise.
fn rejected_detail(outcome: &crate::SessionOutcome) -> String {
    match outcome.actions.as_slice() {
        [
            SessionAction::SendToClient {
                envelope: OutboundMessage::ControlActionResult(result),
                ..
            },
        ] => {
            assert!(!result.accepted, "expected a rejection: {result:?}");
            result.detail.clone()
        }
        other => panic!("expected one rejected result, got {other:?}"),
    }
}

fn bound_engine() -> (SessionEngine, ClientKey, SessionId, Generation) {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    announce(&mut engine, client, session, 1);
    let generation = grant_motion(&mut engine, client);
    (engine, client, session, generation)
}

#[test]
fn a_fully_bound_command_reaches_the_adapter_with_its_correlation_id() {
    let (mut engine, client, session, generation) = bound_engine();
    let outcome = engine.handle_client_message(
        client,
        command(session, generation, ControlAction::Arm, 7),
        MonoTimestamp::from_nanos(10),
    );
    match outcome.actions.as_slice() {
        [SessionAction::ApplyToAdapter { frame, .. }] => {
            assert_eq!(frame.actions, vec![ControlAction::Arm]);
            assert_eq!(frame.action_ids, vec![7]);
            assert_eq!(frame.generation, generation);
            assert_eq!(frame.activation_revision, 1);
            assert!(frame.intent.is_none() && !frame.carries_payload());
        }
        other => panic!("expected adapter delivery, got {other:?}"),
    }
}

#[test]
fn a_zero_correlation_id_is_rejected() {
    let (mut engine, client, session, generation) = bound_engine();
    let outcome = engine.handle_client_message(
        client,
        command(session, generation, ControlAction::Arm, 0),
        MonoTimestamp::from_nanos(10),
    );
    assert!(rejected_detail(&outcome).contains("nonzero"));
}

#[test]
fn a_delayed_arm_bound_to_a_superseded_generation_cannot_re_arm() {
    // The reviewer's interleaving: an Arm was in flight when authority was
    // re-fenced (release + regrant advances the generation past a Disarm).
    // The delayed Arm still carries the OLD generation binding — it must be
    // rejected, never executed.
    let (mut engine, client, session, generation) = bound_engine();
    // Disarm executes under the current generation.
    let disarmed = engine.handle_client_message(
        client,
        command(session, generation, ControlAction::Disarm, 2),
        MonoTimestamp::from_nanos(10),
    );
    assert!(
        matches!(
            disarmed.actions.as_slice(),
            [SessionAction::ApplyToAdapter { .. }]
        ),
        "disarm delivers: {:?}",
        disarmed.actions
    );
    // Authority is re-fenced: release, then reacquire at a fresh generation.
    let released = engine.handle_client_message(
        client,
        DomainEnvelope::Release(LeaseRelease {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        MonoTimestamp::from_nanos(11),
    );
    assert!(!released.actions.is_empty(), "the release acknowledges");
    let fresh = grant_motion(&mut engine, client);
    assert_ne!(fresh, generation, "the fence advanced");
    // The delayed Arm arrives bound to the superseded generation.
    let outcome = engine.handle_client_message(
        client,
        command(session, generation, ControlAction::Arm, 1),
        MonoTimestamp::from_nanos(12),
    );
    assert!(
        rejected_detail(&outcome).contains("stale generation"),
        "the stale-bound arm is refused, not executed"
    );
}

#[test]
fn a_command_from_a_non_holder_is_rejected() {
    let (mut engine, _holder, _session, generation) = bound_engine();
    let other = ClientKey::new(2);
    let other_session = welcome(&mut engine, other);
    announce(&mut engine, other, other_session, 1);
    let outcome = engine.handle_client_message(
        other,
        command(other_session, generation, ControlAction::Arm, 3),
        MonoTimestamp::from_nanos(10),
    );
    assert!(rejected_detail(&outcome).contains("hold"));
}

#[test]
fn an_unannounced_activation_revision_is_rejected() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    // No announcement at all: the binding cannot be validated.
    let generation = grant_motion(&mut engine, client);
    let outcome = engine.handle_client_message(
        client,
        command(session, generation, ControlAction::Arm, 4),
        MonoTimestamp::from_nanos(10),
    );
    assert!(rejected_detail(&outcome).contains("activation revision"));
}

#[test]
fn an_unadvertised_action_is_rejected_before_the_adapter() {
    let (mut engine, client, session, generation) = bound_engine();
    // The test capability advertises Arm only; GimbalRecenter is foreign to
    // the motion scope.
    let outcome = engine.handle_client_message(
        client,
        command(session, generation, ControlAction::GimbalRecenter, 5),
        MonoTimestamp::from_nanos(10),
    );
    assert!(rejected_detail(&outcome).contains("not advertised"));
}

#[test]
fn a_command_naming_a_foreign_session_closes_the_client() {
    let (mut engine, client, session, generation) = bound_engine();
    let forged = SessionId::new(session.as_u64().wrapping_add(9));
    let outcome = engine.handle_client_message(
        client,
        command(forged, generation, ControlAction::Arm, 6),
        MonoTimestamp::from_nanos(10),
    );
    assert!(
        outcome
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::CloseClient { .. })),
        "got {:?}",
        outcome.actions
    );
}

#[test]
fn a_datagram_frame_carrying_typed_actions_is_rejected_whole() {
    use pilotage_protocol::{
        ControlPayload, FrameRejectionReason, ScopedControlFrame, SequenceNum,
    };
    let (mut engine, client, session, generation) = bound_engine();
    let frame = ScopedControlFrame {
        session,
        vehicle: VEHICLE,
        scope: motion(),
        generation,
        sequence: SequenceNum::new(0),
        sampled_at: MonoTimestamp::from_nanos(9),
        profile_revision: 1,
        activation_revision: 1,
        payload: ControlPayload::default(),
        intent: None,
        actions: vec![ControlAction::Arm],
        action_ids: vec![9],
    };
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(frame),
        MonoTimestamp::from_nanos(10),
    );
    match outcome.actions.as_slice() {
        [SessionAction::RejectFrame { rejection, .. }] => {
            assert_eq!(rejection.reason, FrameRejectionReason::ActionOnDatagram);
        }
        other => panic!("expected an action-on-datagram rejection, got {other:?}"),
    }
}

/// A sim reset is a LIFECYCLE action: it is not advertised on any flight
/// scope, and commanding it requires the `sim.lifecycle` scope's OWN lease
/// — flight authority neither grants nor implies it (SIM-01).
#[test]
fn a_sim_reset_needs_the_lifecycle_scopes_own_lease() {
    use pilotage_adapter_api::{SIM_LIFECYCLE_SCOPE, sim_lifecycle_descriptor};

    let mut capabilities = super::capabilities();
    capabilities.vehicles[0]
        .scopes
        .push(sim_lifecycle_descriptor());
    let mut engine = SessionEngine::new(
        capabilities,
        super::staleness(),
        crate::SessionConfig::new(1, "host-test"),
    );
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    announce(&mut engine, client, session, 1);
    let motion_generation = grant_motion(&mut engine, client);

    // Flight authority does not admit the reset: not advertised there.
    let on_flight = engine.handle_client_message(
        client,
        command(session, motion_generation, ControlAction::SimReset, 8),
        MonoTimestamp::from_nanos(10),
    );
    assert!(rejected_detail(&on_flight).contains("not advertised"));

    // Holding flight authority alone does not admit it on the lifecycle
    // scope either — that scope's own lease is required.
    let lifecycle = |generation, id| {
        DomainEnvelope::ActionCommand(ControlActionCommand {
            session,
            vehicle: VEHICLE,
            scope: pilotage_protocol::ScopeId::new(SIM_LIFECYCLE_SCOPE),
            generation,
            activation_revision: 1,
            action: ControlAction::SimReset,
            action_id: id,
        })
    };
    let unleased = engine.handle_client_message(
        client,
        lifecycle(Generation::new(0), 9),
        MonoTimestamp::from_nanos(11),
    );
    assert!(rejected_detail(&unleased).contains("hold"));

    // With the lifecycle lease, the reset is delivered.
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Lease(LeaseRequest {
            vehicle: VEHICLE,
            scope: pilotage_protocol::ScopeId::new(SIM_LIFECYCLE_SCOPE),
        }),
        MonoTimestamp::from_nanos(12),
    );
    let generation = outcome
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: OutboundMessage::LeaseResponse(response),
                ..
            } if response.granted => Some(response.generation),
            _ => None,
        })
        .expect("lifecycle lease granted");
    let delivered = engine.handle_client_message(
        client,
        lifecycle(generation, 10),
        MonoTimestamp::from_nanos(13),
    );
    assert!(
        matches!(
            delivered.actions.as_slice(),
            [SessionAction::ApplyToAdapter { .. }]
        ),
        "got {:?}",
        delivered.actions
    );
}

/// A host whose adapter does not advertise the lifecycle scope (a
/// physical/RF profile) has NO lifecycle authority to lease or command:
/// the scope is structurally absent from the session, not merely refused.
#[test]
fn an_unadvertised_lifecycle_scope_is_structurally_absent() {
    use pilotage_adapter_api::SIM_LIFECYCLE_SCOPE;

    let mut engine = engine(); // the default capabilities carry no lifecycle scope
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    announce(&mut engine, client, session, 1);
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::ActionCommand(ControlActionCommand {
            session,
            vehicle: VEHICLE,
            scope: pilotage_protocol::ScopeId::new(SIM_LIFECYCLE_SCOPE),
            generation: Generation::new(1),
            activation_revision: 1,
            action: ControlAction::SimReset,
            action_id: 11,
        }),
        MonoTimestamp::from_nanos(10),
    );
    assert!(rejected_detail(&outcome).contains("unknown scope"));
}
