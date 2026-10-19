//! The live flight's client: the portable client-session core over a
//! WebTransport connection to the host, and the telemetry it decodes.

use std::time::{Duration, Instant};

use pilotage_client_session::{
    ClientAction, ClientConfig, ClientEngine, ControlCommand, ModuleEvent, MotionDemand,
    ReconnectPolicy, TransportEvent, intent_capability, velocity_intent,
};
use pilotage_protocol::wire;
use tokio::time::timeout;
use wtransport::{ClientConfig as WtClientConfig, Connection, Endpoint};

pub(crate) const STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// The latest telemetry the client decoded.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Reading {
    pub(crate) armed: bool,
    pub(crate) north: f64,
    pub(crate) east: f64,
    pub(crate) up: f64,
    pub(crate) heading: f64,
    pub(crate) tilt_deg: f64,
    pub(crate) roll_rate: f64,
}

pub(crate) struct Client {
    connection: Connection,
    send: wtransport::SendStream,
    recv: wtransport::RecvStream,
    engine: ClientEngine,
    events: Vec<ModuleEvent>,
    reading: Option<Reading>,
    started: Instant,
}

impl Client {
    pub(crate) async fn connect(addr: std::net::SocketAddr) -> Self {
        let config = WtClientConfig::builder()
            .with_bind_default()
            .with_no_cert_validation()
            .build();
        let endpoint = Endpoint::client(config).expect("client endpoint");
        let url = format!("https://127.0.0.1:{}/pilotage", addr.port());
        let connection = timeout(STEP_TIMEOUT, endpoint.connect(url))
            .await
            .expect("connect in time")
            .expect("connects");
        let (send, recv) = timeout(STEP_TIMEOUT, connection.open_bi())
            .await
            .expect("open_bi in time")
            .expect("bootstrap opens")
            .await
            .expect("bootstrap open completes");
        let mut client = Self {
            connection,
            send,
            recv,
            engine: ClientEngine::new(ClientConfig {
                client_name: "live-betaflight-flight".into(),
                reconnect: ReconnectPolicy::default(),
            }),
            events: Vec::new(),
            reading: None,
            started: Instant::now(),
        };
        let actions = client.engine.handle(TransportEvent::Connected, 0);
        client.execute(actions).await;
        client
    }

    pub(crate) async fn execute(&mut self, actions: Vec<ClientAction>) {
        for action in actions {
            match action {
                ClientAction::SendBootstrap(bytes) => {
                    self.send.write_all(&bytes).await.expect("bootstrap write");
                }
                ClientAction::SendDatagram(bytes) => {
                    self.connection.send_datagram(bytes).expect("datagram send");
                }
                ClientAction::Emit(event) => self.observe(event),
                ClientAction::ScheduleReconnect { .. } | ClientAction::Stop(_) => {}
            }
        }
    }

    pub(crate) fn observe(&mut self, event: ModuleEvent) {
        if let ModuleEvent::Telemetry(sample) = &event
            && let (Some(avionics), Some(pose)) = (sample.avionics.as_ref(), sample.pose.as_ref())
        {
            let (x, y) = (f64::from(avionics.quat_x), f64::from(avionics.quat_y));
            self.reading = Some(Reading {
                armed: sample.fc_state.as_ref().is_some_and(|fc| fc.arm_state == 2),
                north: f64::from(avionics.pos_n_m),
                east: f64::from(avionics.pos_e_m),
                up: -f64::from(avionics.pos_d_m),
                heading: f64::from(pose.heading_rad),
                tilt_deg: (1.0 - 2.0 * (x * x + y * y))
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees(),
                roll_rate: f64::from(avionics.rate_p_rad_s),
            });
        }
        self.events.push(event);
    }

    pub(crate) async fn pump_bootstrap(&mut self) {
        let mut buf = vec![0_u8; 16_384];
        let read = timeout(STEP_TIMEOUT, self.recv.read(&mut buf))
            .await
            .expect("bootstrap read in time")
            .expect("bootstrap read")
            .expect("bootstrap open");
        let actions = self
            .engine
            .handle(TransportEvent::BootstrapReceived(buf[..read].to_vec()), 0);
        self.execute(actions).await;
    }

    pub(crate) async fn pump_datagram(&mut self) {
        let datagram = timeout(STEP_TIMEOUT, self.connection.receive_datagram())
            .await
            .expect("telemetry in time")
            .expect("datagram channel open");
        let actions = self.engine.handle(
            TransportEvent::DatagramReceived(datagram.payload().to_vec()),
            0,
        );
        self.execute(actions).await;
    }

    pub(crate) async fn admit_and_lease(&mut self) -> (u64, String) {
        while !std::mem::take(&mut self.events)
            .iter()
            .any(|event| matches!(event, ModuleEvent::Admitted(_)))
        {
            self.pump_bootstrap().await;
        }
        let admission = self.engine.admission().expect("admitted").clone();
        let vehicle = &admission.vehicles[0];
        let scope = vehicle.scopes[0].scope.clone();
        let actions = self.engine.request_lease(vehicle.vehicle_id, &scope);
        self.execute(actions).await;
        while !self.engine.holds_control() {
            self.pump_bootstrap().await;
        }
        (vehicle.vehicle_id, scope)
    }

    /// Sends one discrete action and returns the host's verdict.
    pub(crate) async fn action(&mut self, kind: u32) -> (bool, String) {
        let (vehicle, scope) = self.engine.control_target().expect("holds control");
        let request = wire::ControlActionRequest {
            action: i32::try_from(kind).expect("small"),
            mode_target: 0,
            action_id: 0,
            feel_target: 0,
        };
        let actions = self.engine.control_action(vehicle, &scope, request);
        self.execute(actions).await;
        loop {
            self.pump_bootstrap().await;
            for event in std::mem::take(&mut self.events) {
                if let ModuleEvent::ActionResult(result) = event {
                    return (result.accepted, result.detail);
                }
            }
        }
    }

    /// Flies normalized sticks (roll, pitch, throttle, yaw in the advertised
    /// envelope) until `done` holds or `limit` passes.
    pub(crate) async fn fly(
        &mut self,
        sticks: [f32; 4],
        limit: Duration,
        done: impl FnMut(&Reading) -> bool,
    ) -> Reading {
        let (vehicle, scope) = self.engine.control_target().expect("holds control");
        let [roll, pitch, throttle, yaw] = sticks;
        let intent = velocity_intent(
            MotionDemand {
                roll,
                pitch,
                throttle,
                yaw,
            },
            intent_capability(
                self.engine.admission().expect("admitted"),
                vehicle,
                &scope,
                wire::IntentFamily::Velocity,
            ),
        )
        .expect("the advertised envelope admits a body-frame velocity");
        self.fly_intent(&intent, limit, done).await
    }

    /// Flies one typed intent on the held scope until `done` holds or
    /// `limit` passes.
    pub(crate) async fn fly_intent(
        &mut self,
        intent: &wire::ControlIntent,
        limit: Duration,
        mut done: impl FnMut(&Reading) -> bool,
    ) -> Reading {
        let (vehicle, scope) = self.engine.control_target().expect("holds control");
        let start = Instant::now();
        loop {
            let sampled = u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            let command = ControlCommand::Intent(*intent);
            let actions = self.engine.control_frame(vehicle, &scope, command, sampled);
            self.execute(actions).await;
            self.pump_datagram().await;
            self.events.clear();
            if let Some(reading) = self.reading {
                assert!(reading.tilt_deg < 45.0, "tilted: {reading:?}");
                if done(&reading) || start.elapsed() >= limit {
                    return reading;
                }
            }
        }
    }
}

impl Client {
    /// Swaps the held flight scope for `scope`, as a scheme switch does.
    pub(crate) async fn switch_to(&mut self, scope: &str) {
        let (vehicle, held) = self.engine.control_target().expect("holds control");
        let actions = self.engine.release_lease(vehicle, &held);
        self.execute(actions).await;
        let actions = self.engine.request_lease(vehicle, scope);
        self.execute(actions).await;
        while !self.engine.holds(vehicle, scope) {
            self.pump_bootstrap().await;
        }
    }
}
