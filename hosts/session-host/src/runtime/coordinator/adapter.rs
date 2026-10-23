//! The coordinator's adapter: it exposes no vehicle. A coordinator acts on
//! vehicles only through their own hosts.

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, Disposition, ExecutionMode, LinkLossEnactError,
    LinkLossPolicy, RejectReason, StepBudget, StepOutcome, TelemetryBatch, VehicleAdapter,
    VideoSource,
};
use pilotage_protocol::{ScopeId, ScopedControlFrame, VehicleId};
use pilotage_timing::SimTick;

/// An adapter with no vehicles.
#[derive(Debug, Default)]
pub(crate) struct CoordinatorAdapter;

impl VehicleAdapter for CoordinatorAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            execution: ExecutionMode {
                real_time: true,
                ..ExecutionMode::default()
            },
            vehicles: Vec::new(),
            adapter_version: env!("CARGO_PKG_VERSION").into(),
            control_feel: None,
        }
    }

    fn apply_control(&mut self, _frame: &ScopedControlFrame) -> ApplyOutcome {
        ApplyOutcome {
            tick: SimTick::new(0),
            disposition: Disposition::Rejected(RejectReason::UnknownVehicle),
            action_results: Vec::new(),
        }
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        TelemetryBatch::default()
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        Vec::new()
    }

    fn set_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        _scope: &ScopeId,
        _policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError> {
        Err(LinkLossEnactError::UnknownVehicle { vehicle })
    }

    fn step(&mut self, _budget: StepBudget) -> StepOutcome {
        StepOutcome {
            advanced: 0,
            now: SimTick::new(0),
        }
    }
}
