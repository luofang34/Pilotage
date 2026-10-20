use std::time::Duration;

use pilotage_adapter_api::{LinkLossEnactError, LinkLossPolicy, RejectReason};

use super::{DjiAdapter, capability::MAX_PITCH_RATE};
use crate::GimbalPitchTarget;

const MAX_INTERVAL: Duration = Duration::from_millis(100);

/// Fractional demand is separate from the last complete serial command.
#[derive(Default)]
pub(super) struct BenchState {
    commanded: i16,
    remainder: f64,
    last_input: Option<Duration>,
    recentering: bool,
    write_stalled: bool,
}

impl BenchState {
    pub(super) fn commanded(&self) -> i16 {
        self.commanded
    }

    pub(super) fn discard_elapsed(&mut self, now: Duration) {
        self.last_input = Some(now);
        self.remainder = 0.0;
    }

    pub(super) fn written(&mut self, pitch: GimbalPitchTarget, now: Duration) {
        self.commanded = pitch.tenths();
        if self.write_stalled {
            self.discard_elapsed(now);
        }
        self.write_stalled = false;
        if self.commanded == 0 {
            self.recentering = false;
        }
    }

    pub(super) fn stalled(&mut self, now: Duration, pending: bool) {
        self.write_stalled = pending;
        self.discard_elapsed(now);
    }

    pub(super) fn exclude_io(&mut self, started: Duration, completed: Duration) {
        if let Some(last) = self.last_input
            && let Some(blocked) = completed.checked_sub(started.max(last))
        {
            self.last_input = Some(last.saturating_add(blocked));
        }
    }

    fn demand(&mut self, rate: f32, recenter: bool) -> (f32, bool) {
        if rate != 0.0 && !recenter {
            self.recentering = false;
        }
        let recentering = (recenter || self.recentering) && self.commanded != 0;
        let rate = if recentering {
            MAX_PITCH_RATE
        } else if recenter {
            0.0
        } else {
            rate
        };
        (rate, recentering)
    }

    fn target(&mut self, rate: f32, now: Duration) -> Option<i16> {
        let elapsed = self.last_input.and_then(|last| now.checked_sub(last));
        self.last_input = Some(now);
        let Some(elapsed) = elapsed.filter(|elapsed| *elapsed <= MAX_INTERVAL) else {
            self.remainder = 0.0;
            return None;
        };
        let change = f64::from(rate).to_degrees() * 10.0 * elapsed.as_secs_f64();
        let desired = (f64::from(self.commanded) + self.remainder + change).clamp(-900.0, 0.0);
        let rounded = desired.round() as i16;
        self.remainder = desired - f64::from(rounded);
        (rounded != self.commanded).then_some(rounded)
    }
}

impl DjiAdapter {
    pub(super) fn apply_bench(
        &mut self,
        rate: f32,
        recenter: bool,
        ignored_yaw: bool,
        now: Duration,
    ) -> Result<bool, RejectReason> {
        if !self.movement_ready(now) {
            return Err(RejectReason::MeasurementUnavailable);
        }
        let Some(bench) = &mut self.control.bench else {
            return Err(RejectReason::MeasurementUnavailable);
        };
        let (rate, recentering) = bench.demand(rate, recenter);
        if let Some(pending) = self.connection.pending_pitch() {
            let direction = f32::from(pending.tenths() - bench.commanded).signum();
            if rate == 0.0 || direction != rate.signum() {
                if !self.connection.cancel_movement() {
                    return Err(RejectReason::MeasurementUnavailable);
                }
                bench.discard_elapsed(now);
                bench.write_stalled = false;
            }
        }
        if self.connection.pending_bytes() != 0 && rate != 0.0 {
            bench.stalled(now, self.connection.pending_pitch().is_some());
            return Err(RejectReason::Other(
                "DJI serial packet is still pending".to_owned(),
            ));
        }
        bench.recentering = recentering;
        if rate == 0.0 {
            // The last finite target may still complete. No physical hold is inferred.
            bench.discard_elapsed(now);
            return Ok(ignored_yaw);
        }
        let Some(target) = bench.target(rate, now) else {
            return Ok(true);
        };
        let pitch = GimbalPitchTarget::from_tenths(target)
            .map_err(|error| RejectReason::Other(error.to_string()))?;
        if !self.connection.queue_pitch_until(
            pitch,
            self.control.sequence,
            Some(now.saturating_add(MAX_INTERVAL)),
        ) {
            return Err(RejectReason::Other(
                "DJI serial packet is still pending".to_owned(),
            ));
        }
        self.control.sequence = self.control.sequence.wrapping_add(1);
        Ok(true)
    }

    pub(super) fn bench_link_loss(
        &mut self,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError> {
        if policy.is_some() {
            self.control.link_loss = true;
            self.connection.cancel_movement();
            return Err(LinkLossEnactError::ChannelRejected {
                detail:
                    "one-way DJI bench control is locked; physical neutralization is unavailable"
                        .to_owned(),
            });
        }
        if self.control.link_loss || self.connection.fault.is_some() {
            return Err(LinkLossEnactError::ChannelRejected {
                detail: "one-way DJI bench control needs an explicit restart and horizontal confirmation"
                    .to_owned(),
            });
        }
        Ok(())
    }
}
