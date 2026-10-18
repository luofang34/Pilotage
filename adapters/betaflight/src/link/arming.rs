//! The arm switch: what AUX1 says, and the FC's verdict on each request.
//!
//! Betaflight arms on its ARM box, which AUX1 drives. A request raises the
//! switch; Betaflight either arms or keeps refusing, and the refusal is
//! visible only as "still disarmed". After [`ARM_TIMEOUT`] the switch
//! drops again and the request is reported failed, so a refused arm never
//! leaves the switch high to fire later when a blocker clears.

use std::time::{Duration, Instant};

use pilotage_adapter_api::FcCommandAck;
use pilotage_protocol::CommandOutcome;

/// How long a raised switch waits for Betaflight to arm.
pub(crate) const ARM_TIMEOUT: Duration = Duration::from_secs(2);

/// Where the switch stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Switch {
    Off,
    Arming { deadline: Instant },
    On,
}

/// Request counters. Counters rather than flags, so a request repeated
/// before the link saw the previous one is still one request, and none is
/// lost between two polls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Requests {
    /// Arm requests the adapter accepted.
    pub(crate) arm: u32,
    /// Disarm requests.
    pub(crate) disarm: u32,
    /// Arm requests the adapter refused before the switch moved, because
    /// Betaflight reported a blocker.
    pub(crate) refused: u32,
}

/// The switch state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArmSwitch {
    switch: Switch,
    seen: Requests,
    disarm_pending: bool,
    last_command: Option<FcCommandAck>,
}

impl Default for ArmSwitch {
    fn default() -> Self {
        Self {
            switch: Switch::Off,
            seen: Requests::default(),
            disarm_pending: false,
            last_command: None,
        }
    }
}

impl ArmSwitch {
    /// Folds the latest requests and the FC's arm state in; returns whether
    /// AUX1 should be high.
    pub(crate) fn update(&mut self, requests: Requests, armed: bool, now: Instant) -> bool {
        if requests.refused != self.seen.refused {
            self.record(true, CommandOutcome::TemporarilyRejected);
        }
        let seen = std::mem::replace(&mut self.seen, requests);
        if requests.disarm != seen.disarm {
            self.switch = Switch::Off;
            self.disarm_pending = true;
        } else if requests.arm != seen.arm && !armed {
            self.switch = Switch::Arming {
                deadline: now + ARM_TIMEOUT,
            };
            self.record(true, CommandOutcome::InProgress);
        }
        self.switch = match self.switch {
            Switch::Arming { .. } if armed => {
                self.record(true, CommandOutcome::Accepted);
                Switch::On
            }
            Switch::Arming { deadline } if now >= deadline => {
                self.record(true, CommandOutcome::Failed);
                Switch::Off
            }
            // Betaflight disarmed on its own (failsafe, crash detection):
            // follow it, so the switch must be cycled to arm again.
            Switch::On if !armed => Switch::Off,
            other => other,
        };
        if self.disarm_pending && !armed {
            self.disarm_pending = false;
            self.record(false, CommandOutcome::Accepted);
        }
        self.switch != Switch::Off
    }

    /// The FC's verdict on the most recent request.
    pub(crate) fn last_command(&self) -> Option<FcCommandAck> {
        self.last_command
    }

    fn record(&mut self, arm: bool, outcome: CommandOutcome) {
        self.last_command = Some(FcCommandAck { arm, outcome });
    }
}

#[cfg(test)]
mod tests;
