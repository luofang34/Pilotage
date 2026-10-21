use std::io;
use std::time::Duration;

use pilotage_adapter_api::FlightProcedureReason;

use super::super::observation::Observations;
use super::{Connection, DjiFault, Purpose};

impl Connection {
    pub(in crate::adapter) fn write_pending_blocking(
        &mut self,
        now: Duration,
        movement_ready: bool,
        observations: &Observations,
        completed_at: &mut impl FnMut() -> Duration,
    ) {
        self.flight.poll(now);
        if !self.prepare_pending(now, movement_ready, observations) {
            return;
        }
        let Some(pending) = &self.pending else { return };
        let until = pending
            .expires_at
            .unwrap_or_else(|| now.saturating_add(Duration::from_millis(50)));
        let Some(send_before) = self.origin.checked_add(until) else {
            self.fail(DjiFault::Transport {
                operation: "compute write deadline",
                source: io::ErrorKind::InvalidInput.into(),
            });
            return;
        };
        let Some(transport) = &mut self.transport else {
            return;
        };
        match transport.write(&pending.bytes[pending.offset..], send_before) {
            Ok(0) => self.fail(DjiFault::Transport {
                operation: "write",
                source: io::ErrorKind::WriteZero.into(),
            }),
            Ok(count) => self.record_written_blocking(completed_at(), count),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(source) => {
                if let Some(Purpose::Mode(fence)) =
                    self.pending.as_ref().map(|pending| pending.purpose.clone())
                {
                    self.mode_session.wrote(&fence, completed_at(), false);
                }
                if let Some((_, identity)) = self
                    .pending
                    .as_ref()
                    .and_then(|pending| pending.purpose.pitch())
                {
                    self.execution.failed(identity);
                }
                self.flight.poll(completed_at());
                self.fail(DjiFault::Transport {
                    operation: "write",
                    source,
                });
            }
        }
    }

    fn prepare_pending(
        &mut self,
        now: Duration,
        movement_ready: bool,
        observations: &Observations,
    ) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        match &pending.purpose {
            Purpose::Probe => true,
            Purpose::Stick(write) => {
                let write = *write;
                self.prepare_stick(write, observations, now)
            }
            Purpose::Pitch { .. } => {
                if !movement_ready {
                    self.cancel_movement();
                    return false;
                }
                if pending.offset > 0 && pending.expires_at.is_some_and(|until| now >= until) {
                    self.fail(DjiFault::PartialCommand);
                    return false;
                }
                if let Err(error) = self.execution.prepare(now) {
                    tracing::debug!(%error, "DJI queued target expired before write");
                    self.pending = None;
                    return false;
                }
                true
            }
            Purpose::Flight(fence) => {
                let permitted = self.input_clear()
                    && self.version_observed
                    && self
                        .flight
                        .permits(fence, observations.model_revision(), now)
                    && self
                        .flight
                        .command_cached()
                        .and_then(|command| observations.flight_deadline(command, now))
                        .is_some();
                if !permitted {
                    self.invalidate_flight(FlightProcedureReason::MeasurementUnavailable, now);
                }
                permitted
            }
            Purpose::Mode(fence) => {
                let fence = *fence;
                self.prepare_mode(&fence, observations, now)
            }
        }
    }

    fn record_written_blocking(&mut self, now: Duration, count: usize) {
        let Some(pending) = &mut self.pending else {
            return;
        };
        let Some(actual) = pending
            .bytes
            .get(pending.offset..)
            .and_then(|bytes| bytes.get(..count))
        else {
            self.fail(DjiFault::Transport {
                operation: "invalid write receipt",
                source: io::ErrorKind::InvalidData.into(),
            });
            return;
        };
        let actual = actual.to_vec();
        pending.offset = pending.offset.wrapping_add(count);
        let complete = pending.offset == pending.bytes.len();
        let purpose = pending.purpose.clone();
        self.written_bytes = self.written_bytes.wrapping_add(count as u64);
        if let Purpose::Flight(fence) = &purpose {
            self.flight.wrote(fence, now, complete);
        } else if let Purpose::Mode(fence) = &purpose {
            self.mode_session.wrote(fence, now, false);
        }
        let expired = pending.expires_at.is_some_and(|until| now >= until);
        if complete {
            self.pending = None;
            if let Some((_, identity)) = purpose.pitch()
                && let Err(source) = self.execution.sent(identity, now)
            {
                self.fail(DjiFault::Execution { source });
                return;
            }
        }
        if !self.record_blocking(now, "tx", &actual) {
            return;
        }
        if complete {
            if let Purpose::Stick(write) = &purpose
                && let Some(stream) = &mut self.motion.stream
            {
                stream.written(*write, now);
            }
            self.written_pitch = purpose.pitch();
            if let Purpose::Mode(fence) = &purpose {
                self.mode_session.wrote(fence, now, true);
            }
        }
        if expired && matches!(purpose, Purpose::Mode(_) | Purpose::Stick(_)) {
            self.fail(DjiFault::Transport {
                operation: "mode diagnostic write exceeded its deadline",
                source: io::ErrorKind::TimedOut.into(),
            });
        }
    }
}
