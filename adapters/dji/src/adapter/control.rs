use super::GIMBAL_SCOPE;
use std::time::Duration;

use pilotage_adapter_api::{
    ActionResult, ApplyOutcome, Disposition, LinkLossEnactError, LinkLossPolicy, RejectReason,
};
use pilotage_protocol::{ControlAction, ControlIntent, ScopeId, ScopedControlFrame, VehicleId};

use super::{DjiAdapter, capability::MAX_PITCH_RATE};
use crate::GimbalPitchTarget;

#[derive(Default)]
pub(super) struct ControlState {
    link_loss: bool,
    active_rate: bool,
    recenter_until: Option<Duration>,
    sequence: u16,
}

impl DjiAdapter {
    pub(super) fn apply_at(&mut self, frame: &ScopedControlFrame, now: Duration) -> ApplyOutcome {
        let result = self.validate_and_queue(frame, now);
        match result {
            Ok(transformed) => ApplyOutcome {
                tick: self.tick(),
                disposition: if transformed {
                    Disposition::Transformed
                } else {
                    Disposition::Accepted
                },
                action_results: frame
                    .actions
                    .iter()
                    .cloned()
                    .map(ActionResult::accepted)
                    .collect(),
            },
            Err(reason) => ApplyOutcome::new(self.tick(), Disposition::Rejected(reason)),
        }
    }

    fn validate_and_queue(
        &mut self,
        frame: &ScopedControlFrame,
        now: Duration,
    ) -> Result<bool, RejectReason> {
        if frame.vehicle != self.vehicle {
            return Err(RejectReason::UnknownVehicle);
        }
        if frame.scope.as_str() != GIMBAL_SCOPE {
            return Err(RejectReason::UnknownScope);
        }
        if self.control.link_loss {
            return Err(RejectReason::LinkLossEngaged);
        }
        let (rate, recenter, ignored_yaw) = demand(frame)?;
        if !self.ready(now) {
            return Err(RejectReason::MeasurementUnavailable);
        }
        let Some(pose) = self.observations.latest else {
            return Err(RejectReason::MeasurementUnavailable);
        };
        if !recenter && rate == 0.0 {
            if self.control.recenter_until.is_some_and(|until| now < until)
                || !self.control.active_rate
            {
                return Ok(ignored_yaw);
            }
            if !self.connection.cancel_movement() {
                return Err(RejectReason::MeasurementUnavailable);
            }
        }
        let desired = if recenter {
            0.0
        } else {
            f32::from(pose.angles.pitch_tenths) + rate.to_degrees() * 10.0
        };
        let bounded = desired.round().clamp(-50.0, 0.0) as i16;
        let pitch = GimbalPitchTarget::from_tenths(bounded)
            .map_err(|error| RejectReason::Other(error.to_string()))?;
        if !self.connection.queue_pitch(pitch, self.control.sequence) {
            return Err(RejectReason::Other(
                "DJI serial packet is still pending".to_owned(),
            ));
        }
        self.control.sequence = self.control.sequence.wrapping_add(1);
        self.control.active_rate = rate != 0.0 && !recenter;
        self.control.recenter_until = recenter.then_some(now + Duration::from_secs(1));
        Ok(!recenter || ignored_yaw)
    }

    fn ready(&self, now: Duration) -> bool {
        self.connection.input_clear() && self.observations.ready(now)
    }

    pub(super) fn link_loss_with_time_blocking(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
        mut clock: impl FnMut() -> Duration,
    ) -> Result<(), LinkLossEnactError> {
        if vehicle != self.vehicle {
            return Err(LinkLossEnactError::UnknownVehicle { vehicle });
        }
        if scope.as_str() != GIMBAL_SCOPE {
            return Err(rejected("unknown DJI control scope"));
        }
        if let Some(policy) = policy {
            self.control.link_loss = true;
            if !self.connection.cancel_movement() {
                return Err(rejected("partial DJI command closed the connection"));
            }
            if policy != LinkLossPolicy::Neutralize {
                return Err(rejected("unsupported DJI link-loss policy"));
            }
        }
        // A cached pose cannot authorize a stop or clear a safety latch.
        self.connection
            .read_available_blocking(clock(), &mut self.observations);
        let now = clock();
        if !self.ready(now) {
            return Err(rejected("fresh continuous gimbal observations are absent"));
        }
        if policy.is_none() {
            if self.connection.pending_bytes() != 0 {
                return Err(rejected("a DJI packet is still pending"));
            }
            self.control.link_loss = false;
            return Ok(());
        }
        let Some(pose) = self.observations.latest else {
            return Err(rejected("gimbal observation is absent"));
        };
        let pitch = GimbalPitchTarget::from_tenths(pose.angles.pitch_tenths)
            .map_err(|error| rejected(&error.to_string()))?;
        if !self.connection.queue_pitch(pitch, self.control.sequence) {
            return Err(rejected("a DJI packet is still pending"));
        }
        self.control.sequence = self.control.sequence.wrapping_add(1);
        self.control.active_rate = false;
        self.control.recenter_until = None;
        self.connection.write_pending_blocking(now, true);
        if self.connection.fault.is_some() || self.connection.pending_bytes() != 0 {
            return Err(rejected(
                "the DJI hold command did not reach the complete serial write",
            ));
        }
        Ok(())
    }
}

fn demand(frame: &ScopedControlFrame) -> Result<(f32, bool, bool), RejectReason> {
    if frame.carries_payload()
        || !frame.carries_typed()
        || frame.actions.len() > 1
        || frame
            .actions
            .iter()
            .any(|action| *action != ControlAction::GimbalRecenter)
    {
        return Err(RejectReason::Other(
            "only typed DJI pitch control is supported".to_owned(),
        ));
    }
    let (pitch_rate, ignored_yaw) = match frame.intent {
        Some(ControlIntent::GimbalRate(rate))
            if rate.pitch_rate.is_finite()
                && rate.yaw_rate.is_finite()
                && rate.yaw_rate.abs() <= MAX_PITCH_RATE
                && rate.pitch_rate.abs() <= MAX_PITCH_RATE =>
        {
            // The shared intent uses one bound for both axes. Project it onto
            // pitch so a diagonal stick demand cannot cancel valid pitch control.
            (rate.pitch_rate, rate.yaw_rate != 0.0)
        }
        None => (0.0, false),
        _ => {
            return Err(RejectReason::Other(
                "only finite bounded DJI gimbal rates are supported".to_owned(),
            ));
        }
    };
    Ok((pitch_rate, !frame.actions.is_empty(), ignored_yaw))
}

fn rejected(detail: &str) -> LinkLossEnactError {
    LinkLossEnactError::ChannelRejected {
        detail: detail.to_owned(),
    }
}
