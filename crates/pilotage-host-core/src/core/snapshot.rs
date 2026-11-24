//! A copy of the core, for an embedding host that goes back in time: the
//! host keeps a copy made at one time, and later continues from the copy
//! with the same inputs to get the same outputs.

use pilotage_adapter_aviate::AviateAdapter;

use super::HostCore;
use crate::error::HostCoreError;

impl HostCore {
    /// A second core in the same state, which shares nothing with this
    /// one: the snapshot of the core at the time of its last call. Given
    /// the same calls with the same arguments, the two cores return the
    /// same uplink frames, status and events.
    ///
    /// The copy holds everything the core has not given out yet: the
    /// uplink frames not drained and the events not drained. To go back
    /// in time, the host replaces its core with a copy of the snapshot.
    ///
    /// # Errors
    ///
    /// [`HostCoreError::NotCopyable`] when the adapter refuses its copy.
    /// The embedded adapter that [`Self::new`] builds does not refuse.
    pub fn try_clone(&self) -> Result<Self, HostCoreError> {
        // Every field is named, so a new field does not compile until this
        // function copies it.
        let Self {
            config,
            origin,
            last_now,
            actor,
            principal,
            incoming,
            next_engine_tick,
            next_mission_tick,
            status,
            events,
            counters_logged,
            unknown_ids,
        } = self;
        let actor = actor
            .try_clone_with(AviateAdapter::try_clone)
            .map_err(|source| HostCoreError::NotCopyable {
                source: Box::new(source),
            })?;
        Ok(Self {
            config: config.clone(),
            origin: *origin,
            last_now: *last_now,
            actor,
            principal: principal.clone(),
            incoming: incoming.clone(),
            next_engine_tick: *next_engine_tick,
            next_mission_tick: *next_mission_tick,
            status: status.clone(),
            events: events.clone(),
            counters_logged: *counters_logged,
            unknown_ids: unknown_ids.clone(),
        })
    }
}
