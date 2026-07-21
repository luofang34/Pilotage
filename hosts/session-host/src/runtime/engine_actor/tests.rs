//! The engine actor must actually enact link-loss actions on the adapter —
//! releasing a lease without calling `set_link_loss_policy` would leave the
//! vehicle on its last command (ADR-0008) — and a failed enactment is a
//! counted fail-closed fault, never a silent no-op.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, Disposition, ExecutionMode, LinkLossEnactError,
    LinkLossPolicy, RejectReason, ScopeDescriptor, StepBudget, StepOutcome, TelemetryBatch,
    VehicleAdapter, VehicleDescriptor, VideoSource,
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

/// A STATEFUL vehicle adapter honoring the ADR-0008 latch postcondition: an
/// engage records the latch even when the actuation is refused, a clear removes
/// it ONLY on success, and `apply_control` rejects a latched scope (so a test
/// observes suppression). `fail_enactment` refuses every policy change.
#[derive(Default)]
struct RecordingAdapter {
    link_loss_calls: Vec<(VehicleId, ScopeId, Option<LinkLossPolicy>)>,
    /// Scopes whose latch is currently engaged (suppressing control).
    latched: Vec<ScopeId>,
    fail_enactment: bool,
}

impl VehicleAdapter for RecordingAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        capabilities()
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        let disposition = if self.latched.contains(&frame.scope) {
            Disposition::Rejected(RejectReason::LinkLossEngaged)
        } else {
            Disposition::Accepted
        };
        ApplyOutcome {
            tick: SimTick::new(0),
            disposition,
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
        match policy {
            // Engage records the latch REGARDLESS of the actuation result.
            Some(_) => {
                if !self.latched.contains(scope) {
                    self.latched.push(scope.clone());
                }
                if self.fail_enactment {
                    return Err(LinkLossEnactError::NoActuationChannel);
                }
            }
            // Clear drops the latch ONLY on success (ADR-0008).
            None => {
                if self.fail_enactment {
                    return Err(LinkLossEnactError::NoActuationChannel);
                }
                self.latched.retain(|latched| latched != scope);
            }
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
        retry: false,
    }
}

/// A full-coverage neutral motion frame for the given fencing metadata; the
/// single builder both the raw-frame and recovery-activation helpers use.
fn motion_frame(session: SessionId, generation: Generation, sequence: u32) -> ScopedControlFrame {
    ScopedControlFrame {
        session,
        vehicle: VEHICLE,
        scope: ScopeId::new(MOTION),
        generation,
        sequence: SequenceNum::new(sequence),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        activation_revision: 0,
        payload: ControlPayload {
            axes: vec![(LogicalAxisId::new(0), 0.0)],
            edges: Vec::new(),
        },
        intent: None,
        actions: vec![],
    }
}

/// A raw motion control frame, for driving `apply_control` directly.
fn motion_control_frame() -> ScopedControlFrame {
    motion_frame(SessionId::new(1), Generation::new(1), 1)
}

/// Registers one client and returns its outbound receiver, so a test can
/// observe what the actor broadcasts.
fn register_client(actor: &mut EngineActor<RecordingAdapter>) -> mpsc::Receiver<ToConnection> {
    let (sender, receiver) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
    actor.clients.insert(ClientKey::new(1), sender);
    receiver
}

/// Counts the reliable authority-stream broadcasts (the ack rides one).
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

/// A full-coverage neutral motion frame — the host's recovery activation.
fn neutral_frame(session: SessionId, generation: Generation, sequence: u32) -> DomainEnvelope {
    DomainEnvelope::Frame(motion_frame(session, generation, sequence))
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

/// Drives `actor`'s engine to a fresh holder pending recovery (welcome, grant,
/// release — which latches the adapter — re-grant), enacting every outcome. The
/// client is always `ClientKey::new(1)` (the `register_client` one) so a
/// broadcast ack reaches its receiver.
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
fn a_failed_clear_leaves_the_scope_suppressed() {
    let mut actor = actor();

    // Even a REFUSED engage records the latch, so the scope is suppressed
    // (ADR-0008 fail-closed): apply_control rejects it.
    actor.adapter.fail_enactment = true;
    actor.enact(SessionOutcome {
        actions: vec![engage_action()],
        dropped: 0,
    });
    assert_eq!(
        actor
            .adapter
            .apply_control(&motion_control_frame())
            .disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged),
        "a fenced engage suppresses control even when its actuation was refused"
    );

    // A clear the adapter REFUSES must NOT return the scope to control — the
    // latch drops only on Ok.
    actor.enact(SessionOutcome {
        actions: vec![clear_action(MOTION)],
        dropped: 0,
    });
    assert_eq!(
        actor
            .adapter
            .apply_control(&motion_control_frame())
            .disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged),
        "a refused clear leaves the scope suppressed"
    );

    // A clear the adapter ACCEPTS returns the scope to normal control.
    actor.adapter.fail_enactment = false;
    actor.enact(SessionOutcome {
        actions: vec![clear_action(MOTION)],
        dropped: 0,
    });
    assert_eq!(
        actor
            .adapter
            .apply_control(&motion_control_frame())
            .disposition,
        Disposition::Accepted,
        "a successful clear returns the scope to control"
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
    let _ = authority_messages(&mut client); // drop grant/release/re-grant broadcasts

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

    // The clear is refused: no ack, and the engine holds the scope pending
    // (fail-closed) rather than dropping it, so recovery is not stranded.
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

#[test]
fn a_persistently_refused_clear_is_counted_once_not_per_tick() {
    let mut actor = actor();
    let mut client = register_client(&mut actor);
    let (session, generation) = drive_to_regranted(&mut actor);
    let _ = authority_messages(&mut client);

    // The first (non-retry) clear attempt is refused: counted once.
    actor.adapter.fail_enactment = true;
    let recovered = actor.engine.handle_client_message(
        ClientKey::new(1),
        neutral_frame(session, generation, 1),
        MonoTimestamp::from_nanos(0),
    );
    actor.enact(recovered);
    assert_eq!(
        actor.link_loss_enact_failures, 1,
        "the first refusal is counted once"
    );

    // The engine re-emits the clear every tick; 50 refused RETRIES must add no
    // further faults (no 100 Hz counter/log storm).
    for tick_at in 1..=50 {
        let tick = actor.engine.handle_tick(MonoTimestamp::from_nanos(tick_at));
        actor.enact(tick);
    }
    assert_eq!(
        actor.link_loss_enact_failures, 1,
        "50 refused retries add no faults — counted once, not per tick"
    );
    assert_eq!(
        authority_messages(&mut client),
        0,
        "still no ack while the clear stays refused"
    );
}
