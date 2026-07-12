//! The frozen output contract a manager step produces: a bounded,
//! priority-ordered snapshot of active alerts, the single arbitrated aural
//! command, overflow accounting, and manager health.
//!
//! The alert list is stored in a fixed array and exposed only as a sorted
//! slice, so a consumer can never observe filler entries or an unsorted
//! order. Later visual integration depends on this shape staying stable.

use crate::class::{AlertClass, AlertState, AuralToken};
use crate::condition::AlertId;

/// Most alerts the manager tracks and reports at once. Beyond this,
/// overflow drops the lowest-priority alert, fail-visible.
pub const MAX_ACTIVE_ALERTS: usize = 24;

/// Manager self-health for a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ManagerHealth {
    /// The alerting path is healthy; the output is trustworthy.
    #[default]
    Nominal,
    /// The independent monitor reports the alerting path degraded; the
    /// consumer must also honor primary-data flags directly. The alert list
    /// is still produced and unchanged by this flag.
    Faulted,
}

/// One alert in the manager's output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveAlert {
    /// Stable identity.
    pub id: AlertId,
    /// Severity class.
    pub class: AlertClass,
    /// Acknowledgement-lifecycle state.
    pub state: AlertState,
    /// The static aural token this alert's class owns; the *sounded*
    /// command for the step is [`AlertOutput::aural`], chosen by
    /// arbitration.
    pub aural: AuralToken,
    /// Suppressed by profile inhibition in the current phase. Never true
    /// for a warning.
    pub inhibited: bool,
    /// Hidden by unusual-attitude declutter. Never true for a warning or a
    /// caution.
    pub decluttered: bool,
    /// Generation stamp (wrapping) at the alert's last state change.
    pub generation: u32,
}

impl ActiveAlert {
    const PLACEHOLDER: Self = Self {
        id: AlertId(0),
        class: AlertClass::Status,
        state: AlertState::Acknowledged,
        aural: AuralToken::Silent,
        inhibited: false,
        decluttered: false,
        generation: 0,
    };
}

/// The result of one [`AlertManager::step`](crate::AlertManager::step).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertOutput {
    alerts: [ActiveAlert; MAX_ACTIVE_ALERTS],
    len: usize,
    aural: AuralToken,
    overflow: bool,
    overflow_dropped: u32,
    health: ManagerHealth,
    generation: u32,
}

impl AlertOutput {
    pub(crate) fn empty(generation: u32, overflow_dropped: u32) -> Self {
        Self {
            alerts: [ActiveAlert::PLACEHOLDER; MAX_ACTIVE_ALERTS],
            len: 0,
            aural: AuralToken::Silent,
            overflow: false,
            overflow_dropped,
            health: ManagerHealth::Nominal,
            generation,
        }
    }

    pub(crate) fn set_aural(&mut self, aural: AuralToken) {
        self.aural = aural;
    }

    pub(crate) fn set_overflow(&mut self, overflow: bool) {
        self.overflow = overflow;
    }

    pub(crate) fn set_health(&mut self, health: ManagerHealth) {
        self.health = health;
    }

    pub(crate) fn push(&mut self, alert: ActiveAlert) {
        if self.len < MAX_ACTIVE_ALERTS {
            self.alerts[self.len] = alert;
            self.len += 1;
        }
    }

    pub(crate) fn sort_active(&mut self) {
        self.alerts[..self.len]
            .sort_unstable_by(|a, b| b.class.cmp(&a.class).then(a.id.cmp(&b.id)));
    }

    /// The active alerts, priority-ordered: warning first, ties by
    /// ascending id.
    pub fn active(&self) -> &[ActiveAlert] {
        &self.alerts[..self.len]
    }

    /// The single arbitrated aural command for this step.
    pub fn aural(&self) -> AuralToken {
        self.aural
    }

    /// Whether at least one alert was dropped for capacity this step
    /// (fail-visible).
    pub fn overflow(&self) -> bool {
        self.overflow
    }

    /// Cumulative, wrapping count of alerts ever dropped for capacity.
    pub fn overflow_dropped(&self) -> u32 {
        self.overflow_dropped
    }

    /// Manager self-health for this step.
    pub fn health(&self) -> ManagerHealth {
        self.health
    }

    /// The manager generation (wrapping) after this step.
    pub fn generation(&self) -> u32 {
        self.generation
    }
}
