//! Camera commands are admitted only from actual CAMERA_INFORMATION reports.
use super::Px4Adapter;
use pilotage_adapter_api::{
    ActionResult, ApplyOutcome, Disposition, RejectReason, ScopeDescriptor, TelemetryBatch,
};
use pilotage_mavlink::CameraReport;
use pilotage_protocol::{ControlAction, ScopedControlFrame, wire};
use pilotage_timing::SimTick;
use std::time::Instant;
use tracing::warn;

mod catalog;
mod execution;
mod observation;
#[cfg(test)]
mod tests;
use catalog::{camera_key, camera_scope};
pub(super) use execution::CameraControl;

impl Px4Adapter {
    pub(super) fn camera_scopes(&self) -> Vec<ScopeDescriptor> {
        if self.camera_control.is_none() {
            return Vec::new();
        }
        let incarnation = self
            .estimate
            .as_ref()
            .and_then(|source| source.state.lock().ok())
            .map(|state| state.source_incarnation.into_bytes().to_vec())
            .unwrap_or_default();
        self.camera_reports()
            .into_iter()
            .filter(|(_, report)| report.flags.is_some())
            .filter_map(|(key, report)| match catalog::descriptor(key, &report) {
                Ok(mut descriptor) => {
                    if let Some(payload) = descriptor.payload.as_mut() {
                        payload.connection_incarnation = incarnation.clone();
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

    fn camera_reports(&self) -> Vec<((u8, u8), CameraReport)> {
        self.estimate
            .as_ref()
            .and_then(|source| source.state.lock().ok())
            .map(|state| {
                state
                    .fc_reports
                    .cameras
                    .iter()
                    .map(|(key, report)| (*key, *report))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn expire_camera_reports(&mut self) {
        if let Some(mut state) = self
            .estimate
            .as_ref()
            .and_then(|source| source.state.lock().ok())
        {
            state.expire_camera_reports(Instant::now());
        }
    }

    pub(super) fn maintain_camera(&mut self) {
        self.expire_camera_reports();
        let reports = self.camera_reports();
        if let Some(control) = self.camera_control.as_mut() {
            control.synchronize(&reports, self.started_at);
        }
    }

    pub(super) fn apply_camera(
        &mut self,
        frame: &ScopedControlFrame,
        tick: SimTick,
    ) -> ApplyOutcome {
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
        self.maintain_camera();
        let Some(state) = self.estimate.as_ref().map(|source| source.state.clone()) else {
            return reject("camera receive state unavailable");
        };
        let Some(control) = self.camera_control.as_mut() else {
            return reject("camera transport unavailable");
        };
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
                match control.send(key, command, u64::from(*id), timing, state.clone()) {
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

    pub(super) fn append_camera_samples(&self, batch: &mut TelemetryBatch) {
        let Some(source) = self.estimate.as_ref() else {
            return;
        };
        let Ok(state) = source.state.lock() else {
            return;
        };
        for (key, report) in &state.fc_reports.cameras {
            let zoom = observation::camera_observation(
                *key,
                report,
                state.source_incarnation,
                self.started_at,
            );
            let command = self
                .camera_control
                .as_ref()
                .and_then(|control| control.progress_cached(*key));
            if zoom.is_none() && command.is_none() {
                continue;
            }
            let mut sample =
                pilotage_mavlink::sampling::report_only_sample(self.vehicle, SimTick::new(0));
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
