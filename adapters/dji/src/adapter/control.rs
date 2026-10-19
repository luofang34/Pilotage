use super::GIMBAL_SCOPE;
use std::time::Duration;

use pilotage_adapter_api::{
    ActionResult, ApplyOutcome, Disposition, LinkLossEnactError, LinkLossPolicy, RejectReason,
};
use pilotage_protocol::{ControlAction, ControlIntent, ScopeId, ScopedControlFrame, VehicleId};

use super::{DjiAdapter, DjiControlMode, bench::BenchState, capability::MAX_PITCH_RATE};
use crate::GimbalPitchTarget;

#[derive(Default)]
pub(super) struct ControlState {
    pub(super) link_loss: bool,
    authority: Option<(u64, u64)>,
    active_rate: bool,
    recenter_until: Option<Duration>,
    pub(super) sequence: u16,
    pub(super) bench: Option<BenchState>,
}

impl ControlState {
    pub(super) fn new(mode: DjiControlMode) -> Self {
        Self {
            bench: (mode == DjiControlMode::BenchInitialHorizontal)
                .then(BenchState::from_confirmed_horizontal),
            ..Self::default()
        }
    }

    pub(super) fn mode(&self) -> DjiControlMode {
        if self.bench.is_some() {
            DjiControlMode::BenchInitialHorizontal
        } else {
            DjiControlMode::Observed
        }
    }
}

impl DjiAdapter {
    pub(super) fn apply_at(&mut self, frame: &ScopedControlFrame, now: Duration) -> ApplyOutcome {
        let result = self.validate_and_queue(frame, now);
        if result.is_err()
            && let Some(bench) = &mut self.control.bench
        {
            bench.discard_elapsed(now);
        }
        tracing::debug!(
            elapsed_ns = now.as_nanos(),
            vehicle = self.vehicle.as_u64(),
            scope = frame.scope.as_str(),
            intent = ?frame.intent,
            actions = ?frame.actions,
            outcome = ?result,
            "DJI control request evaluated"
        );
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
        let (rate, recenter) = demand(frame)?;
        let deadline = write_deadline(frame, now)?;
        self.bind_authority(frame, now)?;
        if self.control.bench.is_some() {
            return self.apply_bench(rate, recenter, now, deadline);
        }
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
                return Ok(false);
            }
            if !self.connection.cancel_movement() {
                return Err(RejectReason::MeasurementUnavailable);
            }
        }
        let pitch = turret_dji::observed_pitch_target(pose.angles.pitch_tenths, rate, recenter)
            .map_err(|error| RejectReason::Other(error.to_string()))?;
        if !self
            .connection
            .queue_pitch_until(pitch, self.control.sequence, now, deadline)
        {
            return Err(RejectReason::Other(
                "DJI serial packet is still pending".to_owned(),
            ));
        }
        self.control.sequence = self.control.sequence.wrapping_add(1);
        self.control.active_rate = rate != 0.0 && !recenter;
        self.control.recenter_until = recenter.then_some(now + Duration::from_secs(1));
        Ok(!recenter)
    }

    fn bind_authority(
        &mut self,
        frame: &ScopedControlFrame,
        now: Duration,
    ) -> Result<(), RejectReason> {
        let incoming = (frame.session.as_u64(), frame.generation.as_u64());
        if let Some(previous) = self.control.authority {
            if incoming == previous {
                return Ok(());
            }
            if incoming.0 == previous.0 && incoming.1.wrapping_sub(previous.1) >= (1_u64 << 63) {
                return Err(RejectReason::Other(
                    "DJI control authority generation is stale".to_owned(),
                ));
            }
            if !self.connection.cancel_movement() {
                return Err(RejectReason::MeasurementUnavailable);
            }
            self.control.active_rate = false;
            self.control.recenter_until = None;
            if let Some(bench) = &mut self.control.bench {
                bench.cancel(now);
            }
        }
        self.control.authority = Some(incoming);
        Ok(())
    }

    fn ready(&self, now: Duration) -> bool {
        self.connection.input_clear() && self.observations.ready(now)
    }

    pub(super) fn movement_ready(&self, now: Duration) -> bool {
        if self.control.bench.is_some() {
            !self.control.link_loss && self.connection.input_clear()
        } else {
            self.ready(now)
        }
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
        if self.control.bench.is_some() {
            return self.bench_link_loss(policy);
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
        if !self.connection.queue_pitch_until(
            pitch,
            self.control.sequence,
            now,
            now.saturating_add(turret_dji::MAX_INPUT_INTERVAL),
        ) {
            return Err(rejected("a DJI packet is still pending"));
        }
        self.control.sequence = self.control.sequence.wrapping_add(1);
        self.control.active_rate = false;
        self.control.recenter_until = None;
        self.connection
            .write_pending_blocking(now, true, &mut clock);
        if self.connection.fault.is_some() || self.connection.pending_bytes() != 0 {
            return Err(rejected(
                "the DJI hold command did not reach the complete serial write",
            ));
        }
        Ok(())
    }
}

fn demand(frame: &ScopedControlFrame) -> Result<(f32, bool), RejectReason> {
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
    let pitch_rate = match frame.intent {
        Some(ControlIntent::GimbalRate(rate))
            if rate.pitch_rate.is_finite()
                && rate.yaw_rate.is_finite()
                && rate.yaw_rate == 0.0
                && rate.pitch_rate.abs() <= MAX_PITCH_RATE =>
        {
            rate.pitch_rate
        }
        None => 0.0,
        _ => {
            return Err(RejectReason::Other(
                "only bounded DJI pitch is implemented; yaw and flight control are unavailable"
                    .to_owned(),
            ));
        }
    };
    Ok((pitch_rate, !frame.actions.is_empty()))
}

fn rejected(detail: &str) -> LinkLossEnactError {
    LinkLossEnactError::ChannelRejected {
        detail: detail.to_owned(),
    }
}

fn write_deadline(frame: &ScopedControlFrame, now: Duration) -> Result<Duration, RejectReason> {
    let timing = frame.admitted_timing.ok_or_else(|| {
        RejectReason::Other("DJI control requires an admitted, bounded input lifetime".to_owned())
    })?;
    if !timing.window_bounded {
        return Err(RejectReason::Other(
            "DJI control requires an executor validity window".to_owned(),
        ));
    }
    let mut remaining = timing.send_remaining.min(turret_dji::MAX_INPUT_INTERVAL);
    if let Some(active) = timing.active_remaining {
        remaining = remaining.min(active);
    }
    if remaining.is_zero() {
        return Err(RejectReason::Other(
            "DJI input expired before adapter delivery".to_owned(),
        ));
    }
    Ok(now.saturating_add(remaining))
}
