use std::time::Duration;

use crate::flight::Mini3FlightReport;

#[derive(Default)]
pub(super) struct MotorCycle {
    model: Option<u64>,
    sequence: Option<u16>,
    running: bool,
    stopped: u8,
    received: Option<Duration>,
}

impl MotorCycle {
    pub(super) fn new(model: u64) -> Self {
        Self {
            model: Some(model),
            ..Self::default()
        }
    }

    pub(super) fn observe(&mut self, report: Mini3FlightReport, model: u64, now: Duration) -> bool {
        if self.model != Some(model) {
            *self = Self::default();
            return false;
        }
        if self.sequence.is_some_and(|previous| {
            let delta = report.sequence.wrapping_sub(previous);
            delta == 0 || delta >= 0x8000
        }) {
            return false;
        }
        self.sequence = Some(report.sequence);
        if report.motors_running {
            self.running = true;
            self.stopped = 0;
        } else if !report.is_flying && self.running {
            if self.received.is_none_or(|at| now > at) {
                self.stopped = self.stopped.saturating_add(1);
            }
        } else {
            self.stopped = 0;
        }
        self.received = Some(now);
        self.stopped >= 3
    }
}
