//! Inputs to one manager step: crew and condition events, and the flight
//! context that scopes inhibition and declutter.

use crate::condition::{AlertCondition, AlertId};

/// A single input to [`AlertManager::step`](crate::AlertManager::step).
///
/// Assert and clear carry a typed condition; acknowledgement carries an
/// identity or acknowledges everything asserted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertEvent {
    /// The condition is active as of this step.
    Assert(AlertCondition),
    /// The condition is no longer active.
    Clear(AlertCondition),
    /// The crew acknowledged one alert by identity: silence its aural, keep
    /// its visual per class.
    Acknowledge(AlertId),
    /// The crew acknowledged every currently asserted alert (master
    /// caution/warning press).
    AcknowledgeAll,
}

/// Flight phase, the scope an [`InhibitRule`](crate::InhibitRule) keys on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlightPhase {
    /// On the ground, not taking off.
    #[default]
    Ground,
    /// Takeoff roll and initial climb (the classic inhibit window).
    Takeoff,
    /// Climb.
    Climb,
    /// Cruise.
    Cruise,
    /// Approach.
    Approach,
    /// Landing rollout.
    Landing,
}

/// Caller-supplied context for one step. It scopes inhibition and
/// declutter and reports the independent health of the alerting path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertContext {
    /// Current flight phase; an inhibit rule fires only in its phase.
    pub phase: FlightPhase,
    /// Unusual-attitude declutter is active: hide advisory, status, and
    /// maintenance; retain warnings and cautions.
    pub declutter: bool,
    /// The independent display/alerting-path health input (AIR-IN-013):
    /// `false` marks the output untrusted so the consumer also honors
    /// primary-data flags. It never suppresses the alert list.
    pub alerting_path_healthy: bool,
}

impl Default for AlertContext {
    fn default() -> Self {
        Self {
            phase: FlightPhase::Ground,
            declutter: false,
            alerting_path_healthy: true,
        }
    }
}
