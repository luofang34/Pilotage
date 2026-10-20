use std::time::Duration;

use pilotage_adapter_api::{LinkLossEnactError, LinkLossPolicy, RejectReason};

use super::DjiAdapter;

use turret_dji::PitchPlanner;

pub(super) type BenchState = PitchPlanner;

impl DjiAdapter {
    pub(super) fn apply_bench(
        &mut self,
        rate: f32,
        recenter: bool,
        now: Duration,
        deadline: Duration,
    ) -> Result<bool, RejectReason> {
        if !self.movement_ready(now) {
            return Err(RejectReason::MeasurementUnavailable);
        }
        let Some(bench) = &mut self.control.bench else {
            return Err(RejectReason::MeasurementUnavailable);
        };
        let rate = bench
            .demand(rate, recenter, now)
            .map_err(|error| RejectReason::Other(error.to_string()))?;
        if let Some(pending) = self.connection.pending_pitch() {
            let direction = f32::from(pending.tenths() - bench.commanded()).signum();
            if rate == 0.0 || direction != rate.signum() {
                if !self.connection.cancel_movement() {
                    return Err(RejectReason::MeasurementUnavailable);
                }
                bench.stalled(now, false);
            }
        }
        if self.connection.pending_bytes() != 0 && rate != 0.0 {
            bench.stalled(now, self.connection.pending_pitch().is_some());
            return Err(RejectReason::Other(
                "DJI serial packet is still pending".to_owned(),
            ));
        }
        if rate == 0.0 {
            // The last finite target may still complete. No physical hold is inferred.
            bench.discard_elapsed(now);
            return Ok(false);
        }
        let Some(target) = bench
            .target(rate, now)
            .map_err(|error| RejectReason::Other(error.to_string()))?
        else {
            return Ok(true);
        };
        if !self
            .connection
            .queue_pitch_until(target, self.control.sequence, now, deadline)
        {
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
