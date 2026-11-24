//! A copy of an embedded adapter, for a host that goes back in time. The
//! copy continues from the same state and shares nothing with the
//! original. An adapter that owns a socket, a task, a process clock or a
//! shared-memory mapping has no correct copy: the call refuses it.

use std::sync::{Arc, Mutex};

use super::super::AviateAdapter;
use super::super::sources::EstimateSource;
use crate::error::AviateAdapterError;
use crate::uplink::FlightUplink;

/// Refuses the copy when the adapter owns `part`.
fn refuse(owned: bool, part: &'static str) -> Result<(), AviateAdapterError> {
    if owned {
        return Err(AviateAdapterError::NotCopyable { part });
    }
    Ok(())
}

impl EstimateSource {
    /// A second cache with the same contents.
    fn try_clone(&self) -> Result<Self, AviateAdapterError> {
        #[cfg(feature = "net")]
        refuse(
            self._link.is_some(),
            "the receive task of the estimate link",
        )?;
        // A second owner writes the cache without the adapter: the copy
        // would not get those writes.
        refuse(
            Arc::strong_count(&self.state) != 1,
            "an estimate cache with a second owner",
        )?;
        let state = self
            .state
            .lock()
            .map_err(|_| AviateAdapterError::NotCopyable {
                part: "an estimate cache that a failed writer holds",
            })?
            .clone();
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            #[cfg(feature = "net")]
            _link: None,
        })
    }
}

impl AviateAdapter {
    /// A second adapter in the same state, which shares nothing with this
    /// one. Given the same calls, the two adapters give the same results.
    /// Only an embedded adapter ([`Self::embedded`]) has a copy.
    ///
    /// # Errors
    ///
    /// Returns [`AviateAdapterError::NotCopyable`] when the adapter owns a
    /// socket, a task, a camera producer, a simulation-truth mapping or a
    /// process clock.
    pub fn try_clone(&self) -> Result<Self, AviateAdapterError> {
        // Every field is named, so a new field does not compile until this
        // function copies it or refuses it.
        let Self {
            vehicle,
            profile,
            estimate,
            truth,
            uplink,
            control_feel_profiles,
            control_feel_changed,
            #[cfg(feature = "net")]
            frames,
            camera_bridge,
            navsat_join_failures,
            pointing,
            #[cfg(feature = "net")]
            _frame_forwarder,
            arm,
            arm_incarnation,
            clock,
            started_at,
            last_reset,
            #[cfg(feature = "sim")]
            view_publish_failed,
            link_loss_policy,
            reset_latch,
            fixed_wing,
            #[cfg(test)]
            reset_spawns,
        } = self;
        refuse(truth.is_some(), "a simulation-truth mapping")?;
        refuse(camera_bridge.is_some(), "a camera producer link")?;
        refuse(pointing.is_some(), "the pointing of a camera producer")?;
        #[cfg(feature = "net")]
        refuse(frames.is_some(), "a video frame receiver")?;
        #[cfg(feature = "net")]
        refuse(_frame_forwarder.is_some(), "a video frame task")?;
        let clock = clock.host_set().ok_or(AviateAdapterError::NotCopyable {
            part: "the process clock",
        })?;
        Ok(Self {
            vehicle: *vehicle,
            profile: *profile,
            estimate: estimate
                .as_ref()
                .map(EstimateSource::try_clone)
                .transpose()?,
            truth: None,
            uplink: uplink.as_ref().map(FlightUplink::try_clone).transpose()?,
            control_feel_profiles: control_feel_profiles.clone(),
            control_feel_changed: *control_feel_changed,
            #[cfg(feature = "net")]
            frames: None,
            camera_bridge: None,
            navsat_join_failures: *navsat_join_failures,
            pointing: None,
            #[cfg(feature = "net")]
            _frame_forwarder: None,
            arm: *arm,
            arm_incarnation: *arm_incarnation,
            clock,
            started_at: *started_at,
            last_reset: *last_reset,
            #[cfg(feature = "sim")]
            view_publish_failed: *view_publish_failed,
            link_loss_policy: link_loss_policy.clone(),
            reset_latch: *reset_latch,
            fixed_wing: *fixed_wing,
            #[cfg(test)]
            reset_spawns: *reset_spawns,
        })
    }
}
