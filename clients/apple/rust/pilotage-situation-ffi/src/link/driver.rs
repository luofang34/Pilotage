//! The link's background task: sockets in, engine decisions out.
//!
//! Readers move bytes into one event channel; the driver loop feeds the
//! shared engine, executes the actions it returns, and never interprets
//! a message itself. Reconnect timing is the engine's decision; this
//! task only sleeps until the instant it was given.

use std::sync::Arc;
use std::time::Instant;

use pilotage_client_session::{
    ClientAction, ClientConfig, ClientEngine, ControlCommand, MotionDemand, ProfileIdentity,
    ReconnectPolicy, StreamId, TransportEvent, intent_capability, velocity_intent,
};
use pilotage_control_web::{ControlCoordinator, DEFAULT_PROFILE_BYTES};
use pilotage_instrument_feed::InstrumentFeed;
use pilotage_protocol::wire;
use tokio::sync::mpsc;
use wtransport::{ClientConfig as WtClientConfig, Connection, Endpoint};

use super::LinkCommand;
use super::delivery::DeliveryQueue;
use super::observer::LinkObserver;
use super::records::{LinkConfig, LinkEvent};

/// State-frame cadence: one assembly per display-ish interval. The scene
/// pace is the shell's display link; this only bounds staleness.
const STATE_FRAME_INTERVAL_MS: u64 = 33;

/// The driver's owned state across one connection.
pub(super) struct Link {
    pub(super) engine: ClientEngine,
    /// The shared control runtime: the same profile bytes, curves,
    /// quasimode, and edge logic the browser executes.
    pub(super) control: ControlCoordinator,
    pub(super) feed: Option<InstrumentFeed>,
    pub(super) delivery: DeliveryQueue,
    pub(super) started: Instant,
    pub(super) retry_at_ms: Option<u64>,
    pub(super) stopped: bool,
    pub(super) stats: LinkStats,
    /// Whether the gimbal quasimode captured the stick last tick.
    pub(super) capture_active: bool,
    /// Consecutive pad ticks gated under a held motion lease.
    pub(super) gated_ticks: u32,
}

/// One second of link accounting, reset on report.
#[derive(Debug, Default)]
pub(super) struct LinkStats {
    pub(super) telemetry: u32,
    pub(super) state_frames: u32,
    pub(super) control_frames: u32,
    pub(super) rejected: u32,
    pub(super) action_results: u32,
}

impl Link {
    pub(super) fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Executes engine actions against one live connection.
    async fn execute(
        &mut self,
        actions: Vec<ClientAction>,
        send: &mut wtransport::SendStream,
        connection: &Connection,
    ) {
        for action in actions {
            match action {
                ClientAction::SendBootstrap(bytes) => {
                    if send.write_all(&bytes).await.is_err() {
                        // The reader observes the same loss and reports it.
                    }
                }
                ClientAction::SendDatagram(bytes) => {
                    connection.send_datagram(bytes).ok();
                }
                ClientAction::Emit(event) => self.emit(event),
                ClientAction::ScheduleReconnect { at_ms } => {
                    self.retry_at_ms = Some(at_ms);
                }
                ClientAction::Stop(fault) => {
                    self.stopped = true;
                    self.delivery.event(LinkEvent::Stopped {
                        reason: fault.to_string(),
                    });
                }
            }
        }
    }

    /// Assembles and delivers one state frame when telemetry has been fed.
    fn deliver_state_frame(&mut self) {
        let now_ms = self.now_ms();
        let Some(feed) = self.feed.as_mut() else {
            return;
        };
        let mut buf = vec![0_u8; 4096];
        #[allow(clippy::cast_precision_loss)]
        if let Ok(len) = feed.state_frame(now_ms as f64, &mut buf) {
            buf.truncate(len);
            self.stats.state_frames = self.stats.state_frames.wrapping_add(1);
            self.delivery.state_frame(buf, now_ms);
        }
    }

    /// Reports and resets one second of accounting.
    fn report_stats(&mut self) {
        let stats = std::mem::take(&mut self.stats);
        self.delivery.event(LinkEvent::Stats {
            telemetry_per_second: stats.telemetry,
            state_frames_per_second: stats.state_frames,
            control_frames_per_second: stats.control_frames,
            rejected_per_second: stats.rejected,
            action_results_per_second: stats.action_results,
        });
    }

    /// Builds and sends one fenced motion frame, or nothing without a
    /// lease and an advertised envelope.
    pub(super) fn motion_actions(&mut self, demand: MotionDemand) -> Vec<ClientAction> {
        let Some((vehicle_id, scope)) = self.engine.control_target() else {
            return Vec::new();
        };
        let Some(admission) = self.engine.admission().cloned() else {
            return Vec::new();
        };
        let capability =
            intent_capability(&admission, vehicle_id, &scope, wire::IntentFamily::Velocity);
        let Some(intent) = velocity_intent(demand, capability) else {
            return Vec::new();
        };
        self.stats.control_frames = self.stats.control_frames.wrapping_add(1);
        let sampled_at_nanos = self.now_ms().saturating_mul(1_000_000);
        self.engine
            .control_frame(vehicle_id, &scope, ControlCommand::Intent(intent), sampled_at_nanos)
    }

    /// Builds a discrete action command under the held lease.
    pub(super) fn action_actions(&mut self, code: i32) -> Vec<ClientAction> {
        let Some((vehicle_id, scope)) = self.engine.control_target() else {
            return Vec::new();
        };
        self.engine.control_action(
            vehicle_id,
            &scope,
            wire::ControlActionRequest {
                action: code,
                mode_target: 0,
                action_id: 0,
            },
        )
    }
}

/// Runs the link until shutdown.
pub(crate) async fn run(
    config: LinkConfig,
    pinned: Option<[u8; 32]>,
    observer: Arc<dyn LinkObserver>,
    mut commands: mpsc::UnboundedReceiver<LinkCommand>,
) {
    let mut engine = ClientEngine::new(ClientConfig {
        client_name: config.client_name.clone(),
        reconnect: ReconnectPolicy::default(),
    });
    // The runtime compiles the same built-in profile bytes the browser
    // activates, and the engine announces that runtime's own identity:
    // both clients bind their frames to one mapping, verifiably.
    let mut control = ControlCoordinator::new();
    control.activate_scheme(DEFAULT_PROFILE_BYTES);
    engine.set_profile_identity(ProfileIdentity {
        profile_id: control.profile_id().to_owned(),
        profile_revision: control.profile_revision(),
        activation_revision: control.activation_revision(),
        digest: control.profile_digest(),
    });
    let mut link = Link {
        engine,
        control,
        feed: None,
        delivery: DeliveryQueue::start(observer),
        started: Instant::now(),
        retry_at_ms: None,
        stopped: false,
        stats: LinkStats::default(),
        capture_active: false,
        gated_ticks: 0,
    };
    loop {
        match connect(&config, pinned).await {
            Ok(connection) => {
                if drive(&mut link, &connection, &mut commands).await {
                    return;
                }
            }
            Err(detail) => {
                let now = link.now_ms();
                let actions = link
                    .engine
                    .handle(TransportEvent::TransportLost { detail }, now);
                link.execute_offline(actions);
            }
        }
        if link.stopped {
            return;
        }
        let now = link.now_ms();
        let wait = link
            .retry_at_ms
            .take()
            .map_or(500, |at| at.saturating_sub(now));
        tokio::select! {
            () = tokio::time::sleep(std::time::Duration::from_millis(wait)) => {}
            command = commands.recv() => {
                if matches!(command, Some(LinkCommand::Shutdown) | None) {
                    return;
                }
            }
        }
    }
}

impl Link {
    /// Executes actions when no connection exists: sends are impossible
    /// and are dropped; events and scheduling still apply.
    fn execute_offline(&mut self, actions: Vec<ClientAction>) {
        for action in actions {
            match action {
                ClientAction::Emit(event) => self.emit(event),
                ClientAction::ScheduleReconnect { at_ms } => self.retry_at_ms = Some(at_ms),
                ClientAction::Stop(fault) => {
                    self.stopped = true;
                    self.delivery.event(LinkEvent::Stopped {
                        reason: fault.to_string(),
                    });
                }
                ClientAction::SendBootstrap(_) | ClientAction::SendDatagram(_) => {}
            }
        }
    }
}

/// Opens one WebTransport connection, pinned to the configured
/// certificate hash when one is set.
async fn connect(config: &LinkConfig, pinned: Option<[u8; 32]>) -> Result<Connection, String> {
    let builder = WtClientConfig::builder().with_bind_default();
    let client_config = match pinned {
        Some(digest) => builder
            .with_server_certificate_hashes([wtransport::tls::Sha256Digest::new(digest)])
            .build(),
        None => builder.with_no_cert_validation().build(),
    };
    let endpoint = Endpoint::client(client_config).map_err(|error| error.to_string())?;
    endpoint
        .connect(&config.url)
        .await
        .map_err(|error| error.to_string())
}

/// Reader-task events funneled to the driver loop.
enum ReaderEvent {
    Transport(TransportEvent),
}

/// Drives one live connection; returns `true` on shutdown.
async fn drive(
    link: &mut Link,
    connection: &Connection,
    commands: &mut mpsc::UnboundedReceiver<LinkCommand>,
) -> bool {
    let opened = match connection.open_bi().await {
        Ok(opening) => opening.await.map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };
    let Ok((mut send, recv)) = opened else {
        let now = link.now_ms();
        let actions = link.engine.handle(
            TransportEvent::TransportLost {
                detail: "bootstrap stream failed to open".to_owned(),
            },
            now,
        );
        link.execute_offline(actions);
        return link.stopped;
    };

    let (events_tx, mut events) = mpsc::unbounded_channel::<ReaderEvent>();
    spawn_bootstrap_reader(recv, events_tx.clone());
    spawn_uni_acceptor(connection.clone(), events_tx.clone());
    spawn_datagram_reader(connection.clone(), events_tx);

    let now = link.now_ms();
    let actions = link.engine.handle(TransportEvent::Connected, now);
    link.execute(actions, &mut send, connection).await;

    let mut ticker =
        tokio::time::interval(std::time::Duration::from_millis(STATE_FRAME_INTERVAL_MS));
    let mut stats_ticker = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(ReaderEvent::Transport(event)) = event else {
                    return link.stopped;
                };
                let lost = matches!(event, TransportEvent::TransportLost { .. });
                let now = link.now_ms();
                let actions = link.engine.handle(event, now);
                link.execute(actions, &mut send, connection).await;
                if lost || link.stopped {
                    return link.stopped;
                }
            }
            command = commands.recv() => {
                if handle_command(link, command, &mut send, connection).await {
                    return true;
                }
            }
            _ = ticker.tick() => link.deliver_state_frame(),
            _ = stats_ticker.tick() => link.report_stats(),
        }
    }
}

/// Executes one shell command against the engine; `true` means shutdown.
async fn handle_command(
    link: &mut Link,
    command: Option<LinkCommand>,
    send: &mut wtransport::SendStream,
    connection: &Connection,
) -> bool {
    let actions = match command {
        Some(LinkCommand::RequestLease { vehicle_id, scope }) => {
            link.engine.request_lease(vehicle_id, &scope)
        }
        Some(LinkCommand::ReleaseLease) => match link.engine.control_target() {
            Some((vehicle_id, scope)) => link.engine.release_lease(vehicle_id, &scope),
            None => Vec::new(),
        },
        Some(LinkCommand::Motion {
            roll,
            pitch,
            throttle,
            yaw,
        }) => link.motion_actions(MotionDemand {
            roll,
            pitch,
            throttle,
            yaw,
        }),
        Some(LinkCommand::Action { code }) => link.action_actions(code),
        Some(LinkCommand::PadSample {
            axes,
            values,
            pressed,
        }) => link.pad_actions(&axes, &values, &pressed),
        Some(LinkCommand::SelectPad { id }) => {
            link.control.select_device(&id);
            link.delivery.event(LinkEvent::PadSelected {
                label: link.control.device_label().to_owned(),
                arm_hint: link.control.arm_hint(),
                disarm_hint: link.control.disarm_hint(),
            });
            Vec::new()
        }
        Some(LinkCommand::Takeover { vehicle_id, scope }) => {
            link.engine.request_takeover(vehicle_id, &scope)
        }
        Some(LinkCommand::Offer { to_principal, scope }) => {
            link.engine.offer_transfer(to_principal, &scope)
        }
        Some(LinkCommand::Shutdown) | None => return true,
    };
    link.execute(actions, send, connection).await;
    false
}

/// Reads the bootstrap stream until it ends.
fn spawn_bootstrap_reader(
    mut recv: wtransport::RecvStream,
    events: mpsc::UnboundedSender<ReaderEvent>,
) {
    tokio::spawn(async move {
        let mut buf = vec![0_u8; 8192];
        loop {
            match recv.read(&mut buf).await {
                Ok(Some(read)) => {
                    let event = TransportEvent::BootstrapReceived(buf[..read].to_vec());
                    if events.send(ReaderEvent::Transport(event)).is_err() {
                        return;
                    }
                }
                Ok(None) | Err(_) => {
                    events
                        .send(ReaderEvent::Transport(TransportEvent::TransportLost {
                            detail: "bootstrap stream ended".to_owned(),
                        }))
                        .ok();
                    return;
                }
            }
        }
    });
}

/// Accepts host-initiated uni streams and spawns a reader per stream.
fn spawn_uni_acceptor(connection: Connection, events: mpsc::UnboundedSender<ReaderEvent>) {
    tokio::spawn(async move {
        let mut next_stream = 0_u64;
        loop {
            let Ok(mut recv) = connection.accept_uni().await else {
                return;
            };
            next_stream = next_stream.wrapping_add(1);
            let stream = StreamId(next_stream);
            if events
                .send(ReaderEvent::Transport(TransportEvent::UniStreamOpened(
                    stream,
                )))
                .is_err()
            {
                return;
            }
            let events = events.clone();
            tokio::spawn(async move {
                let mut buf = vec![0_u8; 8192];
                loop {
                    match recv.read(&mut buf).await {
                        Ok(Some(read)) => {
                            let event =
                                TransportEvent::UniStreamReceived(stream, buf[..read].to_vec());
                            if events.send(ReaderEvent::Transport(event)).is_err() {
                                return;
                            }
                        }
                        Ok(None) | Err(_) => {
                            events
                                .send(ReaderEvent::Transport(TransportEvent::UniStreamClosed(
                                    stream,
                                )))
                                .ok();
                            return;
                        }
                    }
                }
            });
        }
    });
}

/// Reads datagrams until the connection ends.
fn spawn_datagram_reader(connection: Connection, events: mpsc::UnboundedSender<ReaderEvent>) {
    tokio::spawn(async move {
        loop {
            match connection.receive_datagram().await {
                Ok(datagram) => {
                    let event = TransportEvent::DatagramReceived(datagram.payload().to_vec());
                    if events.send(ReaderEvent::Transport(event)).is_err() {
                        return;
                    }
                }
                Err(_) => {
                    events
                        .send(ReaderEvent::Transport(TransportEvent::TransportLost {
                            detail: "connection closed".to_owned(),
                        }))
                        .ok();
                    return;
                }
            }
        }
    });
}
