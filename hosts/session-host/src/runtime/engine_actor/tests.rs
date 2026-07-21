//! The engine actor must actually enact link-loss actions on the adapter —
//! releasing a lease without calling `set_link_loss_policy` would leave the
//! vehicle on its last command (ADR-0008) — and a failed enactment is a
//! counted fail-closed fault, never a silent no-op.

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, Disposition, ExecutionMode, LinkLossEnactError,
    LinkLossPolicy, ScopeDescriptor, StepBudget, StepOutcome, TelemetryBatch, VehicleAdapter,
    VehicleDescriptor, VideoSource,
};
use pilotage_protocol::{Generation, LogicalAxisId, ScopeId, ScopedControlFrame, VehicleId};
use pilotage_session::{
    ClientKey, LinkLossTrigger, SessionAction, SessionConfig, SessionEngine, SessionOutcome,
};
use pilotage_timing::{SimTick, StalenessPolicy};
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
fn a_confirmed_clear_acks_the_recovering_client() {
    let mut actor = actor();
    let mut client = register_client(&mut actor);
    actor.enact(SessionOutcome {
        actions: vec![clear_action(MOTION)],
        dropped: 0,
    });
    assert_eq!(
        authority_messages(&mut client),
        1,
        "a confirmed clear must broadcast exactly one LinkLossCleared ack"
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
fn a_failed_clear_retries_until_it_takes_then_acks_exactly_once() {
    let mut actor = actor();
    let mut client = register_client(&mut actor);

    // First attempt is refused: no ack, the fault is counted once, and the
    // clear is held pending — the engine already dropped the engaged marker,
    // so nothing else will re-emit it.
    actor.adapter.fail_enactment = true;
    actor.enact(SessionOutcome {
        actions: vec![clear_action(MOTION)],
        dropped: 0,
    });
    assert_eq!(
        authority_messages(&mut client),
        0,
        "a refused clear does not ack"
    );
    assert_eq!(
        actor.link_loss_enact_failures, 1,
        "the refusal is counted once"
    );

    // The adapter recovers; the next retry tick takes and acks — exactly once.
    actor.adapter.fail_enactment = false;
    actor.retry_pending_clears();
    assert_eq!(
        authority_messages(&mut client),
        1,
        "the deferred clear acks on the first acceptance"
    );

    // Further retries neither re-ack nor re-clear: the pending entry is gone.
    actor.retry_pending_clears();
    assert_eq!(
        authority_messages(&mut client),
        0,
        "exactly one ack across the whole recovery"
    );

    // One refused attempt then one accepted retry both reached the adapter as
    // scope-clears, and the retry was not re-counted as a fault.
    assert_eq!(
        actor.adapter.link_loss_calls,
        vec![
            (VEHICLE, ScopeId::new(MOTION), None),
            (VEHICLE, ScopeId::new(MOTION), None),
        ],
        "a failed clear then a successful retry, both scope-clears"
    );
    assert_eq!(
        actor.link_loss_enact_failures, 1,
        "retries are not re-counted as faults"
    );
}
