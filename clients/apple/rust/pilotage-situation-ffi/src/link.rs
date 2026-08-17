//! The operator-client link: transport driver over the shared session core.
//!
//! Swift owns a socket on no platform. This module compiles the same
//! QUIC/WebTransport client stack the loopback gate uses, drives the
//! portable `pilotage-client-session` engine with it, and feeds admitted
//! telemetry through the shared instrument feed. The Swift shell receives
//! typed events and encoded state frames; it sends demands and lease
//! requests. Every session decision stays in the shared cores (ADR-0032,
//! ADR-0037).

mod driver;
mod events;
mod observer;
mod records;

pub use observer::LinkObserver;
pub use records::{
    LinkCatalog, LinkConfig, LinkEvent, LinkIntentCapability, LinkScope, LinkVehicle,
};

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::error::FfiError;

/// One command from the shell to the driver task.
#[derive(Debug)]
pub(crate) enum LinkCommand {
    RequestLease {
        vehicle_id: u64,
        scope: String,
    },
    ReleaseLease,
    Motion {
        roll: f32,
        pitch: f32,
        throttle: f32,
        yaw: f32,
    },
    Action {
        code: i32,
    },
    Takeover {
        vehicle_id: u64,
        scope: String,
    },
    Offer {
        to_principal: u64,
    },
    Shutdown,
}

/// A running link to one session host.
#[derive(uniffi::Object)]
pub struct LinkSession {
    commands: mpsc::UnboundedSender<LinkCommand>,
    // Owns the driver; dropped on shutdown so the task cannot outlive the
    // object that speaks for it.
    runtime: Option<tokio::runtime::Runtime>,
}

#[uniffi::export]
impl LinkSession {
    /// Connects to a host and drives the session until shutdown. Events
    /// arrive on `observer` from a background task.
    ///
    /// # Errors
    ///
    /// Returns an error when the configuration cannot construct a client
    /// endpoint (a malformed certificate hash, or no runtime).
    #[uniffi::constructor]
    pub fn connect(
        config: LinkConfig,
        observer: Arc<dyn LinkObserver>,
    ) -> Result<Arc<Self>, FfiError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| FfiError::HostLink {
                message: format!("link runtime: {error}"),
            })?;
        let pinned = records::parse_certificate_hash(&config.certificate_sha256_hex)?;
        let (commands, command_rx) = mpsc::unbounded_channel();
        runtime.spawn(driver::run(config, pinned, observer, command_rx));
        Ok(Arc::new(Self {
            commands,
            runtime: Some(runtime),
        }))
    }

    /// Asks for control of one (vehicle, scope). The grant, when it
    /// comes, arrives as an event.
    pub fn request_lease(&self, vehicle_id: u64, scope: String) {
        self.commands
            .send(LinkCommand::RequestLease { vehicle_id, scope })
            .ok();
    }

    /// Stands down from the held scope.
    pub fn release_lease(&self) {
        self.commands.send(LinkCommand::ReleaseLease).ok();
    }

    /// Sends one normalized motion demand. Without a held lease or an
    /// advertised velocity envelope the demand is discarded by the shared
    /// core — unfenced or unadvertised input never leaves the client.
    pub fn send_motion(&self, roll: f32, pitch: f32, throttle: f32, yaw: f32) {
        self.commands
            .send(LinkCommand::Motion {
                roll,
                pitch,
                throttle,
                yaw,
            })
            .ok();
    }

    /// Sends one discrete action under the held lease: 1 arms, 2
    /// disarms (the wire `ControlAction` codes). Without a lease the
    /// action dies in the shared core.
    pub fn send_action(&self, code: i32) {
        self.commands.send(LinkCommand::Action { code }).ok();
    }

    /// Asks the present holder to hand the scope over; the handover
    /// completes without another press if the holder confirms.
    pub fn request_takeover(&self, vehicle_id: u64, scope: String) {
        self.commands
            .send(LinkCommand::Takeover { vehicle_id, scope })
            .ok();
    }

    /// Hands the held scope to the asking principal — the holder's half
    /// of a cooperative handover.
    pub fn offer_transfer(&self, to_principal: u64) {
        self.commands.send(LinkCommand::Offer { to_principal }).ok();
    }

    /// Stops the driver and the connection.
    pub fn shutdown(&self) {
        self.commands.send(LinkCommand::Shutdown).ok();
    }
}

impl Drop for LinkSession {
    fn drop(&mut self) {
        self.commands.send(LinkCommand::Shutdown).ok();
        if let Some(runtime) = self.runtime.take() {
            // The driver task ends on Shutdown; a bounded grace keeps drop
            // from hanging a UI thread if the transport is wedged.
            runtime.shutdown_timeout(std::time::Duration::from_millis(250));
        }
    }
}
