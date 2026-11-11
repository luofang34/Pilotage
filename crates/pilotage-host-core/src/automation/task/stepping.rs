//! How the host that steps the principal drives it: it opens the session,
//! sets the time of each call, takes the commands for the engine, and
//! reads the status, the events and the mission engine.

use pilotage_mission::{MissionEngine, MissionEvent};
use pilotage_protocol::{ClientHello, SESSION_PROTOCOL_VERSION};
use pilotage_session::{ClientKey, DomainEnvelope};

use super::{AutomationStatus, MissionTask};
use crate::engine_actor::ToEngine;

impl MissionTask {
    /// Registers with the engine and opens the session: the commands go
    /// to the outbox for the core to deliver.
    pub fn start(&mut self) -> bool {
        let connected = self.send_command(ToEngine::ClientConnected {
            client: self.client,
        });
        if !connected {
            return false;
        }
        let hello = DomainEnvelope::Hello(ClientHello {
            protocol_version: SESSION_PROTOCOL_VERSION,
            client_name: "mission-executor".to_owned(),
            join_token: Vec::new(),
        });
        self.send_message(hello)
    }

    /// Sets the time of the current call, nanoseconds since the core's
    /// origin.
    pub fn set_now(&mut self, now_ns: u64) {
        self.now_ns = now_ns;
    }

    /// Takes the commands for the engine, in order.
    pub fn take_outbox(&mut self) -> Vec<ToEngine> {
        core::mem::take(&mut self.outbox)
    }

    /// The principal's status.
    pub fn status(&self) -> &AutomationStatus {
        &self.status
    }

    /// The mission engine, once built, to read.
    pub fn mission(&self) -> Option<&MissionEngine> {
        self.mission.as_ref()
    }

    /// The mission engine, once built.
    pub fn mission_mut(&mut self) -> Option<&mut MissionEngine> {
        self.mission.as_mut()
    }

    /// Whether no command waits for the engine.
    pub fn outbox_is_empty(&self) -> bool {
        self.outbox.is_empty()
    }

    /// Takes the mission events, in order.
    pub fn take_events(&mut self) -> Vec<MissionEvent> {
        core::mem::take(&mut self.events)
    }

    /// Makes the principal take the motion scope over from the principal
    /// that holds it: it asks its holder for a transfer instead of asking
    /// for a lease, so the holder keeps commanding the vehicle until the
    /// transfer commits.
    pub fn take_over(&mut self) {
        self.takes_over = true;
    }

    /// Registers the principal under `client` instead of
    /// [`super::MISSION_CLIENT`], as a mission lane's run is.
    pub fn bind_client(&mut self, client: ClientKey) {
        self.client = client;
    }

    /// Ends the principal: it clears the guidance group, so no instrument
    /// keeps a frozen leg on screen after it is gone, and logs its
    /// counters.
    pub fn finish(&mut self) {
        let nanos = self.elapsed_nanos();
        self.publish_nav_guidance(None, nanos);
        self.log_counters();
    }

    /// The principal's client key.
    pub fn client(&self) -> ClientKey {
        self.client
    }
}
