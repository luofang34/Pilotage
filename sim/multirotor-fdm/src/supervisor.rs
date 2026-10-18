//! When to (re)provision, (re)launch, and start feeding Betaflight SITL.
//!
//! Betaflight SITL does not survive every start, and a state packet that
//! arrives too early hurts it:
//!
//! - its TCP serial listener is set up on the main thread while the TCP
//!   thread already runs, and the race sometimes aborts the process before
//!   `bind port 5761` (seen on most starts on a loaded host);
//! - a state packet that arrives before its virtual sensors exist crashes
//!   it, so the model starts streaming only a moment after its init has
//!   bound the MSP port;
//! - some starts never run the PID loop (MSP reports a zero cycle time and
//!   no motor command ever follows the first), so a start counts only once
//!   motor commands flow;
//! - some starts print the MSP port bind and then hold no listening socket,
//!   so a start counts only once that port also accepts a connection.
//!
//! This module is the decision half, as a pure state machine: events in,
//! actions out. The binary runs the processes and the model.

use std::time::{Duration, Instant};

/// How long after its MSP port binds Betaflight gets before the first
/// state packet: the rest of its init (task setup included) follows the
/// bind.
pub const INIT_SETTLE: Duration = Duration::from_secs(1);
/// How long a launched Betaflight may take to finish its init.
pub const INIT_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a streamed Betaflight may take to start sending motor
/// commands before it counts as a start whose PID loop never ran.
pub const MOTOR_TIMEOUT: Duration = Duration::from_secs(5);
/// Motor packets that prove the PID loop runs.
pub const MOTOR_PACKETS_TO_FLY: u64 = 100;
/// Launches allowed between two healthy starts.
pub const MAX_LAUNCHES: u32 = 12;
/// Configuration loads allowed before the configuration counts as broken.
pub const MAX_PROVISIONS: u32 = 6;

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Supervision begins.
    Start,
    /// The configuration-load run exited, successfully or not.
    ProvisionFinished {
        /// Whether it exited with success.
        ok: bool,
    },
    /// Betaflight's init reached its MSP port bind.
    InitComplete,
    /// This many motor packets arrived.
    MotorPackets(u64),
    /// The MSP port probe finished.
    MspProbed {
        /// Whether the port accepted a connection.
        ok: bool,
    },
    /// A flying Betaflight stopped answering MSP (see
    /// [`crate::msp_watch`]).
    MspSilent,
    /// The Betaflight process exited.
    Exited,
    /// Time passed.
    Tick,
}

/// What to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Run Betaflight once to load the configuration file.
    Provision,
    /// Start the Betaflight process.
    Launch,
    /// Reset the model to its landed spawn and start sending state.
    StartStreaming,
    /// Stop sending state: no Betaflight is there to read it.
    StopStreaming,
    /// Kill the Betaflight process; an `Exited` event follows.
    Kill {
        /// Why.
        reason: &'static str,
    },
    /// Check that Betaflight's MSP port accepts a connection; an
    /// `MspProbed` event follows.
    ProbeMsp,
    /// Motor commands flow: Betaflight is flying the model.
    ReportFlying,
    /// A flying Betaflight hung; a restart follows, and the vehicle is back
    /// on the ground at its spawn.
    ReportHang,
    /// Stop supervising and fail.
    GiveUp {
        /// Why.
        reason: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Provisioning,
    Starting { since: Instant },
    Settling { since: Instant },
    Streaming { since: Instant, motors: u64 },
    Probing,
    Flying,
    Stopping,
    GaveUp,
}

/// The supervision state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Supervisor {
    phase: Phase,
    provision: bool,
    provisions: u32,
    launches: u32,
}

impl Supervisor {
    /// A supervisor that loads the configuration first when `provision`.
    #[must_use]
    pub fn new(provision: bool) -> Self {
        Self {
            phase: Phase::Idle,
            provision,
            provisions: 0,
            launches: 0,
        }
    }

    /// Whether Betaflight is flying the model.
    #[must_use]
    pub fn is_flying(&self) -> bool {
        self.phase == Phase::Flying
    }

    /// Folds one event in.
    pub fn handle(&mut self, event: Event, now: Instant) -> Vec<Action> {
        match (self.phase, event) {
            (Phase::GaveUp, _) => Vec::new(),
            (Phase::Idle, Event::Start) if self.provision => self.provision_again(),
            (Phase::Idle, Event::Start) => self.launch(now),
            (Phase::Provisioning, Event::ProvisionFinished { ok: true }) => self.launch(now),
            (Phase::Provisioning, Event::ProvisionFinished { ok: false }) => self.provision_again(),
            (Phase::Starting { .. }, Event::InitComplete) => {
                self.phase = Phase::Settling { since: now };
                Vec::new()
            }
            (Phase::Settling { since }, Event::Tick) if now - since >= INIT_SETTLE => {
                self.phase = Phase::Streaming {
                    since: now,
                    motors: 0,
                };
                vec![Action::StartStreaming]
            }
            (Phase::Starting { since }, Event::Tick) if now - since >= INIT_TIMEOUT => {
                self.stop("Betaflight did not finish its init")
            }
            (Phase::Streaming { since, motors }, Event::MotorPackets(count)) => {
                let motors = motors.saturating_add(count);
                if motors >= MOTOR_PACKETS_TO_FLY {
                    self.phase = Phase::Probing;
                    return vec![Action::ProbeMsp];
                }
                self.phase = Phase::Streaming { since, motors };
                Vec::new()
            }
            (Phase::Probing, Event::MspProbed { ok: true }) => {
                self.phase = Phase::Flying;
                self.launches = 0;
                vec![Action::ReportFlying]
            }
            (Phase::Probing, Event::MspProbed { ok: false }) => {
                self.stop("Betaflight's MSP port does not accept connections")
            }
            (Phase::Flying, Event::MspSilent) => {
                let mut actions = vec![Action::ReportHang];
                actions.extend(self.stop("Betaflight stopped answering MSP"));
                actions
            }
            (Phase::Streaming { since, .. }, Event::Tick) if now - since >= MOTOR_TIMEOUT => {
                self.stop("Betaflight sent no motor commands: its PID loop is not running")
            }
            (
                Phase::Starting { .. }
                | Phase::Settling { .. }
                | Phase::Streaming { .. }
                | Phase::Probing
                | Phase::Flying
                | Phase::Stopping,
                Event::Exited,
            ) => {
                let mut actions = vec![Action::StopStreaming];
                actions.extend(self.launch(now));
                actions
            }
            _ => Vec::new(),
        }
    }

    fn provision_again(&mut self) -> Vec<Action> {
        if self.provisions >= MAX_PROVISIONS {
            self.phase = Phase::GaveUp;
            return vec![Action::GiveUp {
                reason: "Betaflight failed to load the configuration file",
            }];
        }
        self.provisions = self.provisions.wrapping_add(1);
        self.phase = Phase::Provisioning;
        vec![Action::Provision]
    }

    fn launch(&mut self, now: Instant) -> Vec<Action> {
        if self.launches >= MAX_LAUNCHES {
            self.phase = Phase::GaveUp;
            return vec![Action::GiveUp {
                reason: "Betaflight did not start after the allowed launches",
            }];
        }
        self.launches = self.launches.wrapping_add(1);
        self.phase = Phase::Starting { since: now };
        vec![Action::Launch]
    }

    fn stop(&mut self, reason: &'static str) -> Vec<Action> {
        self.phase = Phase::Stopping;
        vec![Action::StopStreaming, Action::Kill { reason }]
    }
}

#[cfg(test)]
mod tests;
