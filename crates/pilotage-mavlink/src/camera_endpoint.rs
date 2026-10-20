//! Camera capability, command, and observation paths shared by MAVLink adapters.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use pilotage_adapter_api::{
    ActionResult, ApplyOutcome, Disposition, RejectReason, ScopeDescriptor, TelemetryBatch,
};
use pilotage_protocol::{ControlAction, ScopeId, ScopedControlFrame, VehicleId, wire};
use pilotage_timing::SimTick;
use tracing::warn;

use crate::{CameraReport, CameraWriteRequirement, LinkState, OutboundCommand};

mod catalog;
mod execution;
mod observation;
#[cfg(test)]
mod tests;

use catalog::{camera_key, camera_scope};
use execution::CameraControl;

/// Each camera component and device has an independent control scope.
pub const SCOPE_PREFIX: &str = "vehicle.camera.";

/// Camera commands require capabilities from actual camera information reports.
#[derive(Debug)]
pub struct CameraEndpoints {
    vehicle: VehicleId,
    state: Arc<Mutex<LinkState>>,
    started_at: Instant,
    control: CameraControl,
}

impl CameraEndpoints {
    /// Binds camera control to the adapter's receive state and command transport.
    #[must_use]
    pub fn new(
        vehicle: VehicleId,
        state: Arc<Mutex<LinkState>>,
        commands: tokio::sync::mpsc::Sender<OutboundCommand>,
        target_system: u8,
        started_at: Instant,
    ) -> Self {
        Self {
            vehicle,
            state,
            started_at,
            control: CameraControl::new(commands, target_system),
        }
    }

    /// Returns scopes from received camera capabilities.
    #[must_use]
    pub fn scopes(&self) -> Vec<ScopeDescriptor> {
        let Ok(state) = self.state.lock() else {
            return Vec::new();
        };
        state
            .fc_reports
            .cameras
            .iter()
            .filter(|(_, report)| report.flags.is_some())
            .filter_map(|(key, report)| match catalog::descriptor(*key, report) {
                Ok(mut descriptor) => {
                    if let Some(payload) = descriptor.payload.as_mut() {
                        payload.connection_incarnation =
                            state.source_incarnation.into_bytes().to_vec();
                    }
                    Some(descriptor)
                }
                Err(error) => {
                    warn!(%error, ?key, "camera capability projection failed");
                    None
                }
            })
            .collect()
    }

    fn maintain(&mut self) {
        let reports = self
            .state
            .lock()
            .ok()
            .map(|mut state| {
                state.expire_camera_reports(Instant::now());
                state
                    .fc_reports
                    .cameras
                    .iter()
                    .map(|(key, report)| (camera_requirement(&state, *key, report), *report))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.control.synchronize(&reports, self.started_at);
    }

    /// Admits typed zoom actions against the discovered camera capability and deadline.
    pub fn apply_control(&mut self, frame: &ScopedControlFrame, tick: SimTick) -> ApplyOutcome {
        let reject = |detail: &str| {
            ApplyOutcome::new(
                tick,
                Disposition::Rejected(RejectReason::Other(detail.to_owned())),
            )
        };
        if frame.vehicle != self.vehicle || frame.intent.is_some() || frame.carries_payload() {
            return reject("invalid camera command binding");
        }
        let Some(key) = camera_key(frame.scope.as_str()) else {
            return reject("unknown camera endpoint");
        };
        let Some(timing) = execution::command_timing(frame, Instant::now()) else {
            return reject("invalid command timing");
        };
        self.maintain();
        let action_results = frame
            .actions
            .iter()
            .enumerate()
            .map(|(index, action)| {
                let ControlAction::CameraZoom { command } = *action else {
                    return ActionResult::rejected(*action, "camera scope accepts zoom only");
                };
                let Some(id) = frame.action_ids.get(index) else {
                    return ActionResult::rejected(*action, "camera command identity missing");
                };
                match self
                    .control
                    .send(key, command, u64::from(*id), timing, self.state.clone())
                {
                    Ok(()) => ActionResult::accepted(*action),
                    Err(error) => ActionResult::rejected(*action, error.to_string()),
                }
            })
            .collect();
        ApplyOutcome {
            tick,
            disposition: Disposition::Accepted,
            action_results,
        }
    }

    /// Appends independent camera observations and command progress, then probes discovery.
    pub fn sample_telemetry(&mut self, batch: &mut TelemetryBatch) {
        self.maintain();
        self.append_samples(batch);
        self.control.probe(Instant::now());
    }

    /// Revokes queued commands for the exact vehicle and camera scope.
    ///
    /// This invalidates local commands. It does not confirm a physical camera stop.
    pub fn invalidate_pending_commands(&mut self, vehicle: VehicleId, scope: &ScopeId) {
        if vehicle == self.vehicle {
            self.control.invalidate(scope.as_str());
        }
    }

    fn append_samples(&self, batch: &mut TelemetryBatch) {
        let Ok(state) = self.state.lock() else { return };
        for (key, report) in &state.fc_reports.cameras {
            let zoom = observation::camera_observation(
                *key,
                report,
                state.source_incarnation,
                self.started_at,
            );
            let command = self
                .control
                .progress_cached(camera_requirement(&state, *key, report));
            if zoom.is_none() && command.is_none() {
                continue;
            }
            let mut sample = crate::sampling::report_only_sample(self.vehicle, SimTick::new(0));
            sample.payload = Some(wire::PayloadState {
                endpoint_id: camera_scope(*key),
                capability_revision: report.capability_revision,
                connection_incarnation: state.source_incarnation.into_bytes().to_vec(),
                connection_epoch: report.epoch,
                zoom,
                command,
                ..Default::default()
            });
            batch.samples.push(sample);
        }
    }
}

fn camera_requirement(
    state: &LinkState,
    key: (u8, u8),
    report: &CameraReport,
) -> CameraWriteRequirement {
    CameraWriteRequirement {
        key,
        epoch: report.epoch,
        revision: report.capability_revision,
        source_id: state.source_id,
        source_incarnation: state.source_incarnation,
        source_epoch: state.source_epoch,
    }
}
