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
    LinkLossTrigger, SessionAction, SessionConfig, SessionEngine, SessionOutcome,
};
use pilotage_timing::{SimTick, StalenessPolicy};
use tokio::time::Instant;

use super::EngineActor;

const VEHICLE: VehicleId = VehicleId::new(1);

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
    link_loss_calls: Vec<(VehicleId, Option<LinkLossPolicy>)>,
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
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError> {
        self.link_loss_calls.push((vehicle, policy));
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
        scope: ScopeId::new("vehicle.motion"),
        generation: Generation::new(2),
        trigger: LinkLossTrigger::HolderSilence,
        policy: LinkLossPolicy::Neutralize,
    }
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
        vec![(VEHICLE, Some(LinkLossPolicy::Neutralize))],
        "EngageLinkLoss must call set_link_loss_policy(Some(Neutralize))"
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
fn clear_link_loss_returns_the_adapter_to_normal_control() {
    let mut actor = actor();
    actor.enact(SessionOutcome {
        actions: vec![SessionAction::ClearLinkLoss { vehicle: VEHICLE }],
        dropped: 0,
    });
    assert_eq!(
        actor.adapter.link_loss_calls,
        vec![(VEHICLE, None)],
        "ClearLinkLoss must call set_link_loss_policy(None)"
    );
}
