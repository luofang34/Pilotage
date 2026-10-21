use super::{AdapterCapabilities, ManagedDji, VehicleAdapter};
use pilotage_adapter_api::{
    ApplyOutcome, Disposition, LinkLossEnactError, LinkLossPolicy, RejectReason, StepBudget,
    StepOutcome, TelemetryBatch, TelemetrySample, VideoSource,
};
use pilotage_protocol::{ScopeId, ScopedControlFrame, VehicleId};
use pilotage_timing::SimTick;

impl ManagedDji {
    fn tick(&self) -> SimTick {
        SimTick::new(self.origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64)
    }

    fn retained_sample(&self) -> TelemetrySample {
        TelemetrySample {
            vehicle: self.vehicle,
            tick: self.tick(),
            flight_procedure: self.retained.clone(),
            payload: None,
            pose: None,
            speed: None,
            avionics: None,
            sim_truth: None,
            fc_state: None,
            gimbal: None,
            battery: None,
        }
    }
}

impl VehicleAdapter for ManagedDji {
    fn capabilities(&self) -> AdapterCapabilities {
        self.adapter
            .as_ref()
            .map_or_else(|| self.offline_capabilities(), VehicleAdapter::capabilities)
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        match &mut self.adapter {
            Some(adapter) => adapter.apply_control(frame),
            None => ApplyOutcome::new(
                self.tick(),
                Disposition::Rejected(RejectReason::LinkLossEngaged),
            ),
        }
    }

    fn invalidate_pending_commands(&mut self, vehicle: VehicleId, scope: &ScopeId) {
        if let Some(adapter) = &mut self.adapter {
            adapter.invalidate_pending_commands(vehicle, scope);
        }
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        let mut batch = self
            .adapter
            .as_mut()
            .map_or_else(TelemetryBatch::default, VehicleAdapter::sample_telemetry);
        if let Some(procedure) = batch
            .samples
            .iter()
            .find_map(|s| s.flight_procedure.as_ref())
        {
            self.retained = Some(procedure.clone());
        } else if self.retained.is_some() {
            if let Some(sample) = batch.samples.first_mut() {
                sample.flight_procedure = self.retained.clone();
            } else {
                batch.samples.push(self.retained_sample());
            }
        }
        batch
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        self.adapter
            .as_ref()
            .map_or_else(Vec::new, VehicleAdapter::video_sources)
    }

    fn set_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError> {
        match &mut self.adapter {
            Some(adapter) => adapter.set_link_loss_policy(vehicle, scope, policy),
            None => Err(LinkLossEnactError::NoActuationChannel),
        }
    }

    fn step(&mut self, budget: StepBudget) -> StepOutcome {
        self.update_blocking();
        if let Some(adapter) = &mut self.adapter {
            adapter.step(budget);
        }
        StepOutcome {
            advanced: 0,
            now: self.tick(),
        }
    }
}
