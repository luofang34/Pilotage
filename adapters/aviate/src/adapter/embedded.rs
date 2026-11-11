//! The embedded adapter: no socket, no task and no clock read. An
//! embedding host (a simulator, a browser page) moves the MAVLink bytes
//! of both links and sets the time before each call; the adapter's
//! decisions are the same as in a process host.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use pilotage_adapter_api::SourceIncarnation;
use pilotage_control_feel::ValidatedFlightFeelProfile;
use pilotage_mavlink::codec::parse_datagram;
use pilotage_mavlink::link::{LinkConfig, LinkState, apply_messages_at};
use pilotage_protocol::VehicleId;
use web_time::Instant;

use super::control_feel::ControlFeelProfiles;
use super::sources::EstimateSource;
use super::startup::validate_adapter_control_feel;
use super::{AviateAdapter, AviateProfile};
use crate::clock::AdapterClock;
use crate::error::AviateAdapterError;
use crate::uplink::FlightUplink;

#[cfg(test)]
mod tests;

/// The identities an embedded adapter stamps its sources with: one for
/// the estimate link and one for the FC state reports on the uplink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddedIncarnations {
    /// The attachment identity of the estimate link.
    pub estimate: SourceIncarnation,
    /// The attachment identity of the FC state reports.
    pub fc_state: SourceIncarnation,
}

impl AviateAdapter {
    /// An adapter for the simulation profile whose two MAVLink links are
    /// byte queues: the host passes each datagram the FC sends on its
    /// telemetry link to [`Self::on_downlink`] and each reply on its
    /// command link to [`Self::on_uplink_reply`], and sends the frames
    /// that [`Self::drain_uplink`] returns to the FC's command port. The
    /// adapter's clock starts at `now` and moves only with
    /// [`Self::set_now`].
    ///
    /// # Errors
    ///
    /// Returns [`AviateAdapterError`] when the control-feel profile does
    /// not match this adapter.
    pub fn embedded(
        vehicle: VehicleId,
        config: LinkConfig,
        control_feel: ValidatedFlightFeelProfile,
        incarnations: EmbeddedIncarnations,
        now: Instant,
    ) -> Result<Self, AviateAdapterError> {
        validate_adapter_control_feel(&control_feel)?;
        let control_feel_profiles = ControlFeelProfiles::new(control_feel.clone())?;
        let mut uplink = FlightUplink::in_memory(control_feel, now)
            .map_err(|source| AviateAdapterError::EmbeddedUplink { source })?;
        uplink.set_expected_source(config.system_id, config.component_id);
        let state = Arc::new(Mutex::new(LinkState::for_source(
            config,
            incarnations.estimate,
            now,
        )));
        Ok(Self {
            vehicle,
            profile: AviateProfile::Simulation,
            estimate: Some(EstimateSource {
                state,
                #[cfg(feature = "net")]
                _link: None,
            }),
            truth: None,
            uplink: Some(uplink),
            control_feel_profiles: Some(control_feel_profiles),
            control_feel_changed: false,
            #[cfg(feature = "net")]
            frames: None,
            camera_bridge: None,
            navsat_join_failures: 0,
            pointing: None,
            #[cfg(feature = "net")]
            _frame_forwarder: None,
            arm: None,
            arm_incarnation: incarnations.fc_state,
            clock: AdapterClock::Manual(now),
            started_at: now,
            last_reset: None,
            #[cfg(feature = "sim")]
            view_publish_failed: false,
            reset_latch: None,
            #[cfg(test)]
            reset_spawns: 0,
            link_loss_policy: BTreeMap::new(),
        })
    }

    /// Sets the time of an embedded adapter and its uplink; a process
    /// adapter keeps reading its own clock.
    pub fn set_now(&mut self, now: Instant) {
        self.clock.set(now);
        if let Some(uplink) = self.uplink.as_mut() {
            uplink.set_now(now);
        }
    }

    /// Folds one datagram from the FC's telemetry link into the estimate
    /// cache at the adapter's current time.
    pub fn on_downlink(&mut self, datagram: &[u8]) {
        let Some(source) = self.estimate.as_ref() else {
            return;
        };
        let mut messages = Vec::new();
        let stats = parse_datagram(datagram, &mut messages);
        apply_messages_at(
            &source.state,
            &messages,
            stats.crc_failures,
            stats.unknown_ids,
            self.clock.now(),
        );
    }

    /// Queues one datagram the FC sent on the command link (heartbeats
    /// to its commander, command acknowledgements) for the next telemetry
    /// sample. False for an adapter whose uplink owns a socket.
    pub fn on_uplink_reply(&mut self, datagram: &[u8]) -> bool {
        self.uplink
            .as_mut()
            .is_some_and(|uplink| uplink.push_fc_reply(datagram))
    }

    /// Takes the frames for the FC's command port sent since the last
    /// call, in send order.
    pub fn drain_uplink(&mut self) -> Vec<Vec<u8>> {
        self.uplink
            .as_mut()
            .map(FlightUplink::take_sent_frames)
            .unwrap_or_default()
    }
}
