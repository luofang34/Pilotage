//! The operator-client link: transport driver over the shared session core.
//!
//! Swift owns a socket on no platform. This module compiles the same
//! QUIC/WebTransport client stack the loopback gate uses, drives the
//! portable `pilotage-client-session` engine with it, and feeds admitted
//! telemetry through the shared instrument feed. The Swift shell receives
//! typed events and encoded state frames; it sends demands and lease
//! requests. Every session decision stays in the shared cores (ADR-0032,
//! ADR-0037).

mod delivery;
mod demand;
mod driver;
mod events;
mod observer;
mod pad;
mod records;
#[cfg(test)]
mod tests;

pub use observer::LinkObserver;
pub use records::{
    LinkCatalog, LinkConfig, LinkControlFeelIdentity, LinkControlFeelMode, LinkEvent,
    LinkIntentCapability, LinkScope, LinkVehicle,
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
    SimReset,
    SelectVideoSource {
        source: u8,
    },
    ArmOrder {
        armed: bool,
    },
    PadSample {
        axes: Vec<f32>,
        values: Vec<f32>,
        pressed: Vec<bool>,
    },
    SelectPad {
        id: String,
    },
    KeyEvent {
        key: String,
        pressed: bool,
    },
    ClearKeys,
    KeySample,
    Takeover {
        vehicle_id: u64,
        scope: String,
    },
    Offer {
        to_principal: u64,
        scope: String,
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

    /// Requests a simulation reset on the host's `sim.lifecycle`
    /// scope. The driver acquires that scope's authority first when it
    /// is not yet held; on a host that does not advertise the action
    /// (not a simulator) nothing leaves the client.
    pub fn request_sim_reset(&self) {
        self.commands.send(LinkCommand::SimReset).ok();
    }

    /// Steers the session's video producer to the named source. The
    /// simulator host renders ONE camera at a time: the payload view
    /// exists while the gimbal scope is engaged, and clears when it is
    /// released — so picking the gimbal source acquires that scope and
    /// aims it (a recenter press), and picking the forward source
    /// releases it. A source the host never advertised changes nothing.
    pub fn select_video_source(&self, source: u8) {
        self.commands
            .send(LinkCommand::SelectVideoSource { source })
            .ok();
    }

    /// Asks the present holder to hand the scope over; the handover
    /// completes without another press if the holder confirms.
    pub fn request_takeover(&self, vehicle_id: u64, scope: String) {
        self.commands
            .send(LinkCommand::Takeover { vehicle_id, scope })
            .ok();
    }

    /// Hands the named held scope to the asking principal — the
    /// holder's half of a cooperative handover.
    pub fn offer_transfer(&self, to_principal: u64, scope: String) {
        self.commands
            .send(LinkCommand::Offer {
                to_principal,
                scope,
            })
            .ok();
    }

    /// Moves the arm order lever. The telegraph sends at most one
    /// command per move and reconciles against the flight controller's
    /// own report; a refusal or a unilateral disarm snaps the lever
    /// back to safe — nothing ever re-arms on its own.
    pub fn set_arm_order(&self, armed: bool) {
        self.commands.send(LinkCommand::ArmOrder { armed }).ok();
    }

    /// Feeds one raw pad sample in Standard Gamepad order (axes:
    /// left X/Y, right X/Y with down positive; buttons in W3C order,
    /// triggers analog). The shared control runtime — the same profile,
    /// curves, quasimode, and edges the browser runs — turns it into
    /// fenced frames, lease plans, and typed arm edges.
    pub fn send_pad_sample(&self, axes: Vec<f32>, values: Vec<f32>, pressed: Vec<bool>) {
        self.commands
            .send(LinkCommand::PadSample {
                axes,
                values,
                pressed,
            })
            .ok();
    }

    /// Resolves a connected pad against the layered profile registry.
    pub fn select_pad(&self, id: String) {
        self.commands.send(LinkCommand::SelectPad { id }).ok();
    }

    /// Records one hardware-keyboard transition. `key` is the canonical
    /// `KeyboardEvent.key` value with single letters lower-cased — the
    /// convention the shared keyboard profile speaks, so the same
    /// bindings drive the browser and this shell.
    pub fn key_event(&self, key: String, pressed: bool) {
        self.commands
            .send(LinkCommand::KeyEvent { key, pressed })
            .ok();
    }

    /// Drops every held key (a text field took focus, the scene left
    /// the foreground, or the keyboard detached), so a key released
    /// out of sight cannot keep flying the vehicle.
    pub fn clear_keys(&self) {
        self.commands.send(LinkCommand::ClearKeys).ok();
    }

    /// Runs one control tick synthesized from the held keys through
    /// the same shared runtime the pad sample rides.
    pub fn send_key_sample(&self) {
        self.commands.send(LinkCommand::KeySample).ok();
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
