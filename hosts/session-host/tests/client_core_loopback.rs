//! The portable client-session core against the real host (ADR-0037).
//!
//! `hosts/session-host/tests/loopback.rs` proves the wire contract with a
//! hand-driven client. This test proves the same contract when every
//! decision is made by `pilotage_client_session::ClientEngine` instead: a
//! minimal transport driver moves bytes and executes the engine's actions,
//! and never composes or interprets a message itself. It is the native
//! headless driver of ADR-0037, in its smallest honest form.
//!
//! Synchronization is event-driven; every wait is a bounded timeout around
//! a protocol response the host must send.

#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use pilotage_client_session::{
    ClientAction, ClientConfig, ClientEngine, ControlCommand, ModuleEvent, ReconnectPolicy,
    StreamId, TransportEvent,
};
use pilotage_protocol::wire;
use pilotage_session_host::cli::AdapterKind;
use pilotage_session_host::runtime;
use tokio::time::timeout;
use wtransport::{ClientConfig as WtClientConfig, Connection, Endpoint};

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// The driver: owns the sockets, executes actions, interprets nothing.
struct Driver {
    connection: Connection,
    send: wtransport::SendStream,
    recv: wtransport::RecvStream,
    authority_recv: Option<wtransport::RecvStream>,
    engine: ClientEngine,
    events: Vec<ModuleEvent>,
}

impl Driver {
    async fn connect(addr: std::net::SocketAddr) -> Self {
        let config = WtClientConfig::builder()
            .with_bind_default()
            .with_no_cert_validation()
            .build();
        let client = Endpoint::client(config).expect("client endpoint constructs");
        let url = format!("https://127.0.0.1:{}/pilotage", addr.port());
        let connection = timeout(TEST_TIMEOUT, client.connect(url))
            .await
            .expect("connect does not time out")
            .expect("client connects to the loopback host");
        let (send, recv) = timeout(TEST_TIMEOUT, connection.open_bi())
            .await
            .expect("open_bi does not time out")
            .expect("bootstrap stream opens")
            .await
            .expect("bootstrap stream finishes opening");
        let mut driver = Self {
            connection,
            send,
            recv,
            authority_recv: None,
            engine: ClientEngine::new(ClientConfig {
                client_name: "client-core-loopback".into(),
                reconnect: ReconnectPolicy::default(),
            }),
            events: Vec::new(),
        };
        let actions = driver.engine.handle(TransportEvent::Connected, 0);
        driver.execute(actions).await;
        driver
    }

    /// Executes engine actions verbatim.
    async fn execute(&mut self, actions: Vec<ClientAction>) {
        for action in actions {
            match action {
                ClientAction::SendBootstrap(bytes) => {
                    self.send
                        .write_all(&bytes)
                        .await
                        .expect("bootstrap write succeeds");
                }
                ClientAction::SendDatagram(bytes) => {
                    self.connection
                        .send_datagram(bytes)
                        .expect("datagram send succeeds");
                }
                ClientAction::Emit(event) => self.events.push(event),
                ClientAction::ScheduleReconnect { .. } | ClientAction::Stop(_) => {}
            }
        }
    }

    /// Pumps one bootstrap-stream read through the engine.
    async fn pump_bootstrap(&mut self) {
        let mut buf = vec![0_u8; 8192];
        let read = timeout(TEST_TIMEOUT, self.recv.read(&mut buf))
            .await
            .expect("bootstrap read does not time out")
            .expect("bootstrap read succeeds")
            .expect("bootstrap stream stays open");
        let actions = self
            .engine
            .handle(TransportEvent::BootstrapReceived(buf[..read].to_vec()), 0);
        self.execute(actions).await;
    }

    /// Accepts the host's session-events uni stream and registers it.
    async fn accept_session_events(&mut self) {
        let recv = timeout(TEST_TIMEOUT, self.connection.accept_uni())
            .await
            .expect("accept_uni does not time out")
            .expect("session-events stream is accepted");
        self.authority_recv = Some(recv);
        let actions = self
            .engine
            .handle(TransportEvent::UniStreamOpened(StreamId(1)), 0);
        self.execute(actions).await;
    }

    /// Pumps one session-events read through the engine.
    async fn pump_session_events(&mut self) {
        let recv = self
            .authority_recv
            .as_mut()
            .expect("session-events stream registered");
        let mut buf = vec![0_u8; 8192];
        let read = timeout(TEST_TIMEOUT, recv.read(&mut buf))
            .await
            .expect("session-events read does not time out")
            .expect("session-events read succeeds")
            .expect("session-events stream stays open");
        let actions = self.engine.handle(
            TransportEvent::UniStreamReceived(StreamId(1), buf[..read].to_vec()),
            0,
        );
        self.execute(actions).await;
    }

    /// Pumps one datagram through the engine.
    async fn pump_datagram(&mut self) {
        let datagram = timeout(TEST_TIMEOUT, self.connection.receive_datagram())
            .await
            .expect("datagram wait does not time out")
            .expect("datagram channel stays open");
        let actions = self.engine.handle(
            TransportEvent::DatagramReceived(datagram.payload().to_vec()),
            0,
        );
        self.execute(actions).await;
    }

    fn take_events(&mut self) -> Vec<ModuleEvent> {
        std::mem::take(&mut self.events)
    }

    /// Pumps the bootstrap stream until the engine is admitted, then
    /// returns the admission.
    async fn await_admission(&mut self) -> pilotage_client_session::Admission {
        while !self
            .take_events()
            .iter()
            .any(|event| matches!(event, ModuleEvent::Admitted(_)))
        {
            self.pump_bootstrap().await;
        }
        self.engine
            .admission()
            .expect("admission is retained")
            .clone()
    }

    /// Requests a lease through the engine and pumps until it is held.
    async fn acquire_lease(&mut self, vehicle_id: u64, scope: &str) {
        let actions = self.engine.request_lease(vehicle_id, scope);
        self.execute(actions).await;
        while !self.engine.holds_control() {
            self.pump_bootstrap().await;
        }
    }

    /// Sends fenced full-throttle frames until telemetry reports movement.
    async fn drive_until_moving(&mut self) {
        let throttle = wire::ControlPayload {
            axes: vec![
                wire::AxisSample {
                    axis_id: u32::from(pilotage_adapter_reference::THROTTLE_AXIS),
                    value: 1.0,
                },
                wire::AxisSample {
                    axis_id: u32::from(pilotage_adapter_reference::STEERING_AXIS),
                    value: 0.0,
                },
            ],
            edges: Vec::new(),
        };
        let deadline = tokio::time::Instant::now() + TEST_TIMEOUT;
        let mut speed = 0.0_f64;
        while speed <= 0.0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "telemetry must report the applied frame before the timeout"
            );
            let actions = self
                .engine
                .control_frame(ControlCommand::Legacy(throttle.clone()), 1);
            self.execute(actions).await;
            self.pump_datagram().await;
            for event in self.take_events() {
                if let ModuleEvent::Telemetry(sample) = event {
                    speed = sample
                        .velocity
                        .as_ref()
                        .map_or(0.0, |velocity| f64::from(velocity.linear_x_mps));
                }
            }
        }
    }
}

#[tokio::test]
async fn the_client_core_drives_admission_lease_and_an_applied_control_frame() {
    let host = runtime::start_with_options(
        0,
        AdapterKind::Reference,
        runtime::RuntimeOptions {
            legacy_compatibility: true,
            mission: None,
        },
    )
    .await
    .expect("host starts on an ephemeral port");

    let mut driver = Driver::connect(host.local_addr).await;

    // Admission: the engine sent hello on Connected; the welcome admits.
    let admission = driver.await_admission().await;
    assert!(
        admission.offers_control(),
        "the reference host offers control"
    );
    let vehicle_id = admission.vehicles[0].vehicle_id;
    let scope = admission.vehicles[0].scopes[0].scope.clone();

    driver.accept_session_events().await;
    driver.acquire_lease(vehicle_id, &scope).await;

    // The authority stream announces the same grant to every observer.
    while driver
        .engine
        .authority()
        .holder(vehicle_id, &scope)
        .and_then(|h| h.holder_id)
        .is_none()
    {
        driver.pump_session_events().await;
    }
    assert_eq!(
        driver
            .engine
            .authority()
            .holder(vehicle_id, &scope)
            .and_then(|h| h.holder_id),
        Some(admission.principal_id),
        "the authority mirror sees this principal as the holder"
    );

    // Arm through the fenced reliable action path. The result must come
    // back accepted: an unannounced profile activation would reject it,
    // which is exactly the fault this leg pins down.
    let actions = driver.engine.control_action(wire::ControlActionRequest {
        action: 1,
        mode_target: 0,
        action_id: 0,
    });
    driver.execute(actions).await;
    let deadline = tokio::time::Instant::now() + TEST_TIMEOUT;
    let mut armed = false;
    while !armed {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the arm result must arrive before the timeout"
        );
        driver.pump_bootstrap().await;
        for event in driver.take_events() {
            if let ModuleEvent::ActionResult(result) = event {
                assert!(result.accepted, "arm must be accepted: {}", result.detail);
                armed = true;
            }
        }
    }

    // Control: full throttle through the engine's fenced lane, applied by
    // the reference adapter and observed as nonzero speed in telemetry.
    driver.drive_until_moving().await;

    host.shutdown().await;
}
