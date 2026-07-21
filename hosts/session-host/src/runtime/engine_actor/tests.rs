//! The engine actor must actually enact link-loss actions on the adapter —
//! releasing a lease without calling `set_link_loss_policy` would leave the
//! vehicle on its last command (ADR-0008) — and a failed enactment is a
//! counted fail-closed fault, never a silent no-op.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, Disposition, ExecutionMode, LinkLossEnactError,
    LinkLossPolicy, ScopeDescriptor, StepBudget, StepOutcome, TelemetryBatch, VehicleAdapter,
    VehicleDescriptor, VideoSource,
};
use pilotage_protocol::{
    ClientHello, ControlPayload, Generation, LeaseRelease, LeaseRequest, LogicalAxisId, ScopeId,
    ScopedControlFrame, SequenceNum, SessionId, VehicleId,
};
use pilotage_session::{
    ClientKey, DomainEnvelope, LinkLossTrigger, OutboundMessage, SessionAction, SessionConfig,
    SessionEngine, SessionOutcome,
};
use pilotage_timing::{MonoTimestamp, SimTick, StalenessPolicy};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::EngineActor;
use crate::runtime::connection::ToConnection;
use crate::runtime::registry::OUTBOUND_QUEUE_CAPACITY;

const VEHICLE: VehicleId = VehicleId::new(1);
const MOTION: &str = "vehicle.motion";

fn capabilities() -> AdapterCapabilities {
    AdapterCapabilities {
        execution: ExecutionMode {
            real_time: true,
            deterministic: true,
            ..ExecutionMode::default()
        },
        vehicles: vec![VehicleDescriptor {
            id: VEHICLE,
            scopes: vec![ScopeDescriptor {
                scope: ScopeId::new("vehicle.motion"),
                axes: vec![LogicalAxisId::new(0)],
            }],
            link_loss_actions: vec![LinkLossPolicy::Neutralize],
        }],
        adapter_version: "test".to_owned(),
    }
}

/// A vehicle adapter that records only its `set_link_loss_policy` calls; every
/// other trait method is an inert stub, since the enactment path under test
/// touches only the link-loss policy. With `fail_enactment` set it refuses
/// every policy change, exercising the fail-closed fault path.
#[derive(Default)]
struct RecordingAdapter {
    link_loss_calls: Vec<(VehicleId, ScopeId, Option<LinkLossPolicy>)>,
    fail_enactment: bool,
}

impl VehicleAdapter for RecordingAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        capabilities()
    }

    fn apply_control(&mut self, _frame: &ScopedControlFrame) -> ApplyOutcome {
        ApplyOutcome {
            tick: SimTick::new(0),
            disposition: Disposition::Accepted,
        }
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        TelemetryBatch {
            samples: Vec::new(),
        }
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        Vec::new()
    }

    fn set_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError> {
        self.link_loss_calls.push((vehicle, scope.clone(), policy));
        if self.fail_enactment {
            return Err(LinkLossEnactError::NoActuationChannel);
        }
        Ok(())
    }

    fn step(&mut self, _budget: StepBudget) -> StepOutcome {
        StepOutcome {
            advanced: 0,
            now: SimTick::new(0),
        }
    }
}

fn actor() -> EngineActor<RecordingAdapter> {
    let engine = SessionEngine::new(
        capabilities(),
        StalenessPolicy::new(std::time::Duration::from_millis(250)),
        SessionConfig::new(1, "host-test"),
    );
    EngineActor::new(engine, RecordingAdapter::default(), Instant::now())
}

fn engage_action() -> SessionAction {
    SessionAction::EngageLinkLoss {
        vehicle: VEHICLE,
        scope: ScopeId::new(MOTION),
        generation: Generation::new(2),
        trigger: LinkLossTrigger::HolderSilence,
        policy: LinkLossPolicy::Neutralize,
    }
}

fn clear_action(scope: &str) -> SessionAction {
    SessionAction::ClearLinkLoss {
        vehicle: VEHICLE,
        scope: ScopeId::new(scope),
        generation: Generation::new(3),
    }
}

/// Registers one client and returns its outbound receiver, so a test can
/// observe what the actor broadcasts (the recovery ack rides this channel).
fn register_client(actor: &mut EngineActor<RecordingAdapter>) -> mpsc::Receiver<ToConnection> {
    let (sender, receiver) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
    actor.clients.insert(ClientKey::new(1), sender);
    receiver
}

/// The recovery ack travels on the reliable authority stream; count only those
/// broadcasts so unrelated traffic never masquerades as an ack.
fn authority_messages(receiver: &mut mpsc::Receiver<ToConnection>) -> usize {
    let mut count = 0;
    while let Ok(message) = receiver.try_recv() {
        if matches!(message, ToConnection::AuthorityMessage(_)) {
            count += 1;
        }
    }
    count
}

fn hello() -> DomainEnvelope {
    DomainEnvelope::Hello(ClientHello {
        protocol_version: 1,
        client_name: "host-test".to_owned(),
        join_token: Vec::new(),
    })
}

fn lease() -> DomainEnvelope {
    DomainEnvelope::Lease(LeaseRequest {
        vehicle: VEHICLE,
        scope: ScopeId::new(MOTION),
    })
}

fn release() -> DomainEnvelope {
    DomainEnvelope::Release(LeaseRelease {
        vehicle: VEHICLE,
        scope: ScopeId::new(MOTION),
    })
}

/// A full-coverage neutral motion frame at `generation` — the recovery
/// activation the host requires to clear the scope's latch.
fn neutral_frame(session: SessionId, generation: Generation, sequence: u32) -> DomainEnvelope {
    DomainEnvelope::Frame(ScopedControlFrame {
        session,
        vehicle: VEHICLE,
        scope: ScopeId::new(MOTION),
        generation,
        sequence: SequenceNum::new(sequence),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        payload: ControlPayload {
            axes: vec![(LogicalAxisId::new(0), 0.0)],
            edges: Vec::new(),
        },
    })
}

fn welcome_session(outcome: &SessionOutcome) -> SessionId {
    outcome
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: OutboundMessage::Welcome(welcome),
                ..
            } => Some(welcome.session),
            _ => None,
        })
        .expect("a welcome")
}

fn grant_generation(outcome: &SessionOutcome) -> Generation {
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
        .expect("a granted lease")
}

/// Drives `actor`'s engine to a fresh holder pending recovery — welcome, grant,
/// release (which engages link-loss and latches the adapter), re-grant —
/// enacting every outcome. Returns the session and re-granted generation so the
/// caller can send the neutral activation and observe the recovery ack. The
/// recovering client is always `ClientKey::new(1)` (the one `register_client`
/// registers), so a broadcast ack reaches its receiver.
fn drive_to_regranted(actor: &mut EngineActor<RecordingAdapter>) -> (SessionId, Generation) {
    let client = ClientKey::new(1);
    let now = MonoTimestamp::from_nanos(0);
    let welcome = actor.engine.handle_client_message(client, hello(), now);
    let session = welcome_session(&welcome);
    actor.enact(welcome);
    let granted = actor.engine.handle_client_message(client, lease(), now);
    actor.enact(granted);
    let released = actor.engine.handle_client_message(client, release(), now);
    actor.enact(released); // engages link-loss → the adapter's motion latch is set
    let regranted = actor.engine.handle_client_message(client, lease(), now);
    let generation = grant_generation(&regranted);
    actor.enact(regranted);
    (session, generation)
}

#[test]
fn engage_link_loss_neutralizes_the_adapter() {
    let mut actor = actor();
    actor.enact(SessionOutcome {
        actions: vec![engage_action()],
        dropped: 0,
    });
    assert_eq!(
        actor.adapter.link_loss_calls,
        vec![(
            VEHICLE,
            ScopeId::new(MOTION),
            Some(LinkLossPolicy::Neutralize)
        )],
        "EngageLinkLoss must call set_link_loss_policy(scope, Some(Neutralize))"
    );
    assert_eq!(actor.link_loss_enact_failures, 0);
}

#[test]
fn a_failed_enactment_is_a_counted_fault() {
    let mut actor = actor();
    actor.adapter.fail_enactment = true;
    actor.enact(SessionOutcome {
        actions: vec![engage_action()],
        dropped: 0,
    });
    assert_eq!(
        actor.link_loss_enact_failures, 1,
        "a refused policy change must be counted, never silent"
    );
}

#[test]
fn clear_link_loss_returns_the_scope_to_normal_control() {
    let mut actor = actor();
    actor.enact(SessionOutcome {
        actions: vec![clear_action(MOTION)],
        dropped: 0,
    });
    assert_eq!(
        actor.adapter.link_loss_calls,
        vec![(VEHICLE, ScopeId::new(MOTION), None)],
        "ClearLinkLoss must call set_link_loss_policy(scope, None)"
    );
}

#[test]
fn recovery_acks_once_when_the_adapter_confirms_the_clear() {
    let mut actor = actor();
    let mut client = register_client(&mut actor);
    let (session, generation) = drive_to_regranted(&mut actor);
    // Drop the grant/release/re-grant authority broadcasts; only the recovery
    // ack should follow.
    let _ = authority_messages(&mut client);

    let recovered = actor.engine.handle_client_message(
        ClientKey::new(1),
        neutral_frame(session, generation, 1),
        MonoTimestamp::from_nanos(0),
    );
    actor.enact(recovered);
    assert_eq!(
        authority_messages(&mut client),
        1,
        "a confirmed recovery broadcasts exactly one LinkLossCleared ack"
    );
    assert!(
        actor
            .adapter
            .link_loss_calls
            .contains(&(VEHICLE, ScopeId::new(MOTION), None)),
        "the adapter's motion latch was cleared"
    );
}

#[test]
fn a_failed_clear_emits_no_ack_and_counts_the_fault() {
    let mut actor = actor();
    actor.adapter.fail_enactment = true;
    let mut client = register_client(&mut actor);
    actor.enact(SessionOutcome {
        actions: vec![clear_action(MOTION)],
        dropped: 0,
    });
    assert_eq!(
        authority_messages(&mut client),
        0,
        "a clear the adapter refused must NOT ack — the client keeps neutralizing"
    );
    assert_eq!(
        actor.link_loss_enact_failures, 1,
        "a refused clear is a counted fail-closed fault, never silent"
    );
}

#[test]
fn a_refused_clear_is_retried_by_the_engine_and_acks_once_when_it_takes() {
    let mut actor = actor();
    let mut client = register_client(&mut actor);
    let (session, generation) = drive_to_regranted(&mut actor);
    let _ = authority_messages(&mut client);

    // The neutral activation requests the clear, but the adapter refuses it: no
    // ack, and the engine holds the scope pending (fail-closed) rather than
    // dropping it — so recovery is not stranded.
    actor.adapter.fail_enactment = true;
    let recovered = actor.engine.handle_client_message(
        ClientKey::new(1),
        neutral_frame(session, generation, 1),
        MonoTimestamp::from_nanos(0),
    );
    actor.enact(recovered);
    assert_eq!(
        authority_messages(&mut client),
        0,
        "a refused clear does not ack"
    );
    assert_eq!(actor.link_loss_enact_failures, 1, "the refusal is counted");

    // The adapter recovers; the engine re-emits the clear on the next tick, it
    // takes, and the ack fires — exactly once.
    actor.adapter.fail_enactment = false;
    let tick = actor.engine.handle_tick(MonoTimestamp::from_nanos(1));
    actor.enact(tick);
    assert_eq!(
        authority_messages(&mut client),
        1,
        "the engine-driven retry acks once when the adapter takes the clear"
    );

    // A further tick neither re-clears nor re-acks: the scope is Cleared.
    let tick = actor.engine.handle_tick(MonoTimestamp::from_nanos(2));
    actor.enact(tick);
    assert_eq!(
        authority_messages(&mut client),
        0,
        "exactly one ack across the whole recovery"
    );
}
