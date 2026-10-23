//! Hold and resume: a human takes the motion scope, the mission waits,
//! and its sequencing clock pauses so phase deadlines do not count the
//! time a human flew.

use navigate_contract::{DurationNanos, MonotonicNanos};

use super::{MissionEngine, MissionEvent, MissionState};

impl MissionEngine {
    /// Holds the mission: a human took the motion scope. Ticks command
    /// nothing and the sequencing clock pauses until [`Self::resume`].
    /// A terminal mission and a mission already held are unchanged.
    pub fn hold(&mut self, now: MonotonicNanos) {
        if self.hold_started.is_some()
            || matches!(self.state(), MissionState::Complete | MissionState::Failed)
        {
            return;
        }
        self.hold_started = Some(now);
        self.hold_events.push(MissionEvent::MissionHeld);
    }

    /// Resumes a held mission from the active leg. The time held leaves
    /// the sequencing clock. Returns the hold's length, or `None` when the
    /// mission was not held.
    pub fn resume(&mut self, now: MonotonicNanos) -> Option<DurationNanos> {
        let started = self.hold_started.take()?;
        let held = now
            .elapsed_since(started)
            .unwrap_or(DurationNanos::from_nanos(0));
        self.held_total_ns = self.held_total_ns.saturating_add(held.as_nanos());
        self.hold_events.push(MissionEvent::MissionResumed {
            held_ns: held.as_nanos(),
        });
        Some(held)
    }

    /// The plan's progress: the index of the waypoint the plan flies
    /// toward, and whether its last waypoint is captured.
    #[must_use]
    pub fn plan_progress(&self) -> (usize, bool) {
        let total = self.execution.plan().waypoints.len();
        let remaining = self.execution.remaining_waypoints().len();
        (total.saturating_sub(remaining), self.plan_complete)
    }

    /// Whether a human holds the motion scope and the mission waits.
    #[must_use]
    pub fn is_held(&self) -> bool {
        self.hold_started.is_some()
    }

    /// The sequencing clock: the engine clock less every completed hold.
    /// Phase deadlines count on this clock; the wall deadline does not.
    pub(super) fn sequencing_time(&self, now: MonotonicNanos) -> MonotonicNanos {
        MonotonicNanos::from_nanos(now.as_nanos().saturating_sub(self.held_total_ns))
    }
}
