//! The pilot's session with the host: the client engine on the link.
//!
//! The pilot is a client like the browser and the tablet. It announces
//! its own profile identity, so the host's records name the language-model
//! pilot as the source of its frames. It takes a lease through the same
//! authority path, and it gives the lease up when another principal asks.

use std::time::{Duration, Instant};

use pilotage_client_session::{
    ClientAction, ClientConfig, ClientEngine, ControlCommand, ModuleEvent, MotionDemand,
    ProfileIdentity, ReconnectPolicy, TransportEvent, intent_capability, velocity_intent,
};
use pilotage_protocol::wire;
use tokio::sync::mpsc;

use crate::error::PilotError;
use crate::link::Link;
use pilotage_agent::{Demand, Discrete};

/// The profile identity this control source announces.
pub(crate) const PROFILE_ID: &str = "automation.intent-pilot/v1";
/// The scope that carries vehicle motion on every adapter.
pub(crate) const MOTION_SCOPE: &str = "vehicle.motion";
/// Deadline for admission and for the lease, in seconds.
const HANDSHAKE_TIMEOUT_S: u64 = 10;
/// Time to wait for the host's next word after the release, in seconds. A
/// close right after the write can drop the release with the connection.
const RELEASE_WORD_TIMEOUT_S: u64 = 2;

/// An admitted session that holds the motion lease.
pub(crate) struct Session {
    link: Link,
    engine: ClientEngine,
    events: mpsc::Receiver<TransportEvent>,
    started: Instant,
    vehicle_id: u64,
    velocity: wire::IntentCapability,
    actions: Vec<wire::ActionCapability>,
}

impl Session {
    /// Connects, waits for admission, and takes the motion lease.
    pub(crate) async fn open(url: &str, cert_sha256: [u8; 32]) -> Result<Self, PilotError> {
        let (link, events) = Link::connect(url, cert_sha256).await?;
        let mut engine = ClientEngine::new(ClientConfig {
            client_name: "intent-pilot".to_owned(),
            reconnect: ReconnectPolicy::default(),
        });
        engine.set_profile_identity(ProfileIdentity {
            digest: pilotage_input::content_digest(PROFILE_ID.as_bytes()),
            profile_id: PROFILE_ID.to_owned(),
            profile_revision: 1,
            activation_revision: 1,
        });
        let mut session = Self {
            link,
            engine,
            events,
            started: Instant::now(),
            vehicle_id: 0,
            velocity: wire::IntentCapability::default(),
            actions: Vec::new(),
        };
        let connected = session.engine.handle(TransportEvent::Connected, 0);
        session.execute(connected).await?;
        session.admit().await?;
        session.take_lease().await?;
        Ok(session)
    }

    /// Waits for the next transport event and returns what it means.
    /// `None` means the event queue closed.
    pub(crate) async fn next_events(&mut self) -> Option<Result<Vec<ModuleEvent>, PilotError>> {
        let event = self.events.recv().await?;
        Some(self.ingest(event).await)
    }

    /// Advertised speed at full horizontal demand, in metres per second.
    pub(crate) fn max_linear_mps(&self) -> f64 {
        f64::from(self.velocity.max_linear)
    }

    /// True when the motion scope advertises `request`.
    pub(crate) fn offers(&self, request: Discrete) -> bool {
        let wanted = wire_action(request) as i32;
        self.actions.iter().any(|offered| offered.action == wanted)
    }

    /// True while this session holds the motion lease.
    pub(crate) fn holds_control(&self) -> bool {
        self.engine.holds(self.vehicle_id, MOTION_SCOPE)
    }

    /// Sends one fenced velocity frame for `demand`.
    pub(crate) async fn send_demand(&mut self, demand: Demand) -> Result<(), PilotError> {
        // The core has a demand type of its own, so it needs no session crate.
        let demand = MotionDemand {
            roll: demand.roll,
            pitch: demand.pitch,
            throttle: demand.throttle,
            yaw: demand.yaw,
        };
        let Some(intent) = velocity_intent(demand, Some(&self.velocity)) else {
            return Err(PilotError::Unoffered {
                detail: "the advertised velocity envelope has no body frame".to_owned(),
            });
        };
        let sampled_at = u64::try_from(self.started.elapsed().as_nanos())
            .unwrap_or(u64::MAX)
            .max(1);
        let actions = self.engine.control_frame(
            self.vehicle_id,
            MOTION_SCOPE,
            ControlCommand::Intent(intent),
            sampled_at,
        );
        self.execute(actions).await
    }

    /// Sends one discrete request on the reliable action channel.
    pub(crate) async fn send_action(&mut self, request: Discrete) -> Result<(), PilotError> {
        let action = wire_action(request);
        let actions = self.engine.control_action(
            self.vehicle_id,
            MOTION_SCOPE,
            wire::ControlActionRequest {
                action: action as i32,
                ..wire::ControlActionRequest::default()
            },
        );
        self.execute(actions).await
    }

    /// Gives the lease to the principal that asked for it. A person who
    /// wants the vehicle gets it; the pilot does not contest.
    pub(crate) async fn yield_to(&mut self, principal: u64) -> Result<(), PilotError> {
        let actions = self.engine.offer_transfer(principal, MOTION_SCOPE);
        self.execute(actions).await
    }

    /// Releases the lease and closes the link. The close waits for the next
    /// word of the host on authority: it comes on the same stream after the
    /// release, so the release was read. Without the wait, a close can drop
    /// the release with the connection, and the host sees a link loss.
    pub(crate) async fn close(mut self) {
        let actions = self.engine.release_lease(self.vehicle_id, MOTION_SCOPE);
        if let Err(error) = self.execute(actions).await {
            tracing::warn!(%error, "the lease release did not reach the host");
        }
        let wait = Duration::from_secs(RELEASE_WORD_TIMEOUT_S);
        match tokio::time::timeout(wait, self.next_authority_word()).await {
            Ok(Ok(word)) => tracing::info!(word, "the host answered after the release"),
            Ok(Err(error)) => tracing::debug!(%error, "the link ended before a word"),
            Err(_) => tracing::warn!(
                "no word from the host after the release; the link-loss policy covers the vehicle"
            ),
        }
        self.link.shutdown().await;
    }

    /// The next authority event or lease response from the host.
    async fn next_authority_word(&mut self) -> Result<&'static str, PilotError> {
        loop {
            let Some(event) = self.events.recv().await else {
                return Err(PilotError::TransportLost {
                    detail: "the event queue closed before the host answered".to_owned(),
                });
            };
            for event in self.ingest(event).await? {
                match event {
                    ModuleEvent::Authority(_) => return Ok("authority event"),
                    ModuleEvent::Lease(_) => return Ok("lease response"),
                    _ => {}
                }
            }
        }
    }

    async fn admit(&mut self) -> Result<(), PilotError> {
        let admission = self
            .pump_until("admission", |engine| engine.admission().is_some())
            .await
            .and_then(|()| {
                self.engine
                    .admission()
                    .cloned()
                    .ok_or(PilotError::HostTimeout {
                        step: "admission",
                        seconds: HANDSHAKE_TIMEOUT_S,
                    })
            })?;
        let unoffered = |detail: &str| PilotError::Unoffered {
            detail: detail.to_owned(),
        };
        // The scope is chosen by name. An adapter lists several scopes,
        // and the first one in the list is not always the motion scope.
        let vehicle = admission
            .vehicles
            .iter()
            .find(|vehicle| vehicle.scopes.iter().any(|s| s.scope == MOTION_SCOPE))
            .ok_or_else(|| unoffered("no vehicle advertises vehicle.motion"))?;
        self.vehicle_id = vehicle.vehicle_id;
        self.actions = vehicle
            .scopes
            .iter()
            .find(|scope| scope.scope == MOTION_SCOPE)
            .map(|scope| scope.actions.clone())
            .unwrap_or_default();
        self.velocity = intent_capability(
            &admission,
            vehicle.vehicle_id,
            MOTION_SCOPE,
            wire::IntentFamily::Velocity,
        )
        .cloned()
        .ok_or_else(|| unoffered("vehicle.motion advertises no velocity intent"))?;
        tracing::info!(
            vehicle = self.vehicle_id,
            max_linear = self.velocity.max_linear,
            max_vertical = self.velocity.max_vertical,
            max_angular = self.velocity.max_angular,
            "admitted"
        );
        Ok(())
    }

    async fn take_lease(&mut self) -> Result<(), PilotError> {
        let actions = self.engine.request_lease(self.vehicle_id, MOTION_SCOPE);
        self.execute(actions).await?;
        let vehicle_id = self.vehicle_id;
        self.pump_until("motion lease", |engine| {
            engine.holds(vehicle_id, MOTION_SCOPE)
        })
        .await
    }

    async fn pump_until(
        &mut self,
        step: &'static str,
        done: impl Fn(&ClientEngine) -> bool,
    ) -> Result<(), PilotError> {
        let deadline = Duration::from_secs(HANDSHAKE_TIMEOUT_S);
        let pump = async {
            while !done(&self.engine) {
                let Some(event) = self.events.recv().await else {
                    return Err(PilotError::TransportLost {
                        detail: format!("the event queue closed during {step}"),
                    });
                };
                for event in self.ingest(event).await? {
                    if let ModuleEvent::Lease(response) = event
                        && !response.granted
                    {
                        return Err(PilotError::Unoffered {
                            detail: format!(
                                "the host denied the lease, reason {}",
                                response.reason
                            ),
                        });
                    }
                }
            }
            Ok(())
        };
        tokio::time::timeout(deadline, pump)
            .await
            .map_err(|_| PilotError::HostTimeout {
                step,
                seconds: HANDSHAKE_TIMEOUT_S,
            })?
    }

    async fn ingest(&mut self, event: TransportEvent) -> Result<Vec<ModuleEvent>, PilotError> {
        if let TransportEvent::TransportLost { detail } = &event {
            return Err(PilotError::TransportLost {
                detail: detail.clone(),
            });
        }
        let now_ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let actions = self.engine.handle(event, now_ms);
        let mut emitted = Vec::new();
        self.execute_into(actions, &mut emitted).await?;
        Ok(emitted)
    }

    async fn execute(&mut self, actions: Vec<ClientAction>) -> Result<(), PilotError> {
        let mut unread = Vec::new();
        self.execute_into(actions, &mut unread).await
    }

    async fn execute_into(
        &mut self,
        actions: Vec<ClientAction>,
        emitted: &mut Vec<ModuleEvent>,
    ) -> Result<(), PilotError> {
        for action in actions {
            match action {
                ClientAction::SendBootstrap(bytes) => self.link.send_bootstrap(&bytes).await?,
                ClientAction::SendDatagram(bytes) => self.link.send_datagram(&bytes)?,
                ClientAction::Emit(event) => emitted.push(event),
                // The pilot flies one scenario on one connection. A lost
                // connection ends the run; it does not reconnect to a
                // vehicle whose state it no longer knows.
                ClientAction::ScheduleReconnect { .. } => {}
                ClientAction::Stop(fault) => {
                    return Err(PilotError::TransportLost {
                        detail: fault.to_string(),
                    });
                }
            }
        }
        Ok(())
    }
}

fn wire_action(request: Discrete) -> wire::ControlAction {
    match request {
        Discrete::Arm => wire::ControlAction::Arm,
        Discrete::Disarm => wire::ControlAction::Disarm,
    }
}
