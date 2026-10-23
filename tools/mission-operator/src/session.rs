//! One operator session with a host: admission, commands, receipts and the
//! lane's status, each written as one JSON line. The desk that resolves
//! commands into intents is the one every client uses.

use std::io::Write as _;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pilotage_client_link::Link;
use pilotage_client_session::mission::{
    ApprovalPolicy, FiledReceipt, MissionCommand, MissionDesk, ProposalView, ReceiptView,
    SurveyFile,
};
use pilotage_client_session::{
    ClientAction, ClientConfig, ClientEngine, ModuleEvent, ReconnectPolicy, TransportEvent,
};
use pilotage_protocol::wire;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::cli::{Action, Command, OperatorError};

mod agent;
mod survey_agent;

/// How long a receipt may take: a team answers after its members do.
const RECEIPT_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a new session waits for the lane's first status; lanes
/// publish at least every two seconds.
const FIRST_STATUS: Duration = Duration::from_secs(5);

struct Session {
    link: Link,
    transport: mpsc::Receiver<TransportEvent>,
    engine: ClientEngine,
    desk: MissionDesk,
    policy: ApprovalPolicy,
    started: Instant,
    /// Status lines go out on every status, or only on a change.
    every_status: bool,
    last_shown: Option<(Value, Instant)>,
    last_telemetry: Option<Instant>,
}

pub(crate) async fn execute(command: Command) -> Result<bool, OperatorError> {
    let mut session = Session::open(&command).await?;
    let deadline = Instant::now() + Duration::from_secs(command.seconds);
    let outcome = match command.action {
        Action::Send(mission) => session
            .ask(&mission)
            .await
            .map(|r| r.is_some_and(|r| r.accepted())),
        Action::Watch => session.watch(deadline).await,
        Action::Run(objective) => session.run(objective, deadline).await,
        Action::Agent => {
            session.every_status = false;
            session.agent().await
        }
        Action::SurveyAgent(objective) => {
            session.every_status = false;
            session.survey_agent(objective, deadline).await
        }
    };
    session.link.shutdown().await;
    outcome
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl Session {
    async fn open(command: &Command) -> Result<Self, OperatorError> {
        let (link, transport) = Link::connect(&command.url, command.cert_sha256)
            .await
            .map_err(OperatorError::Link)?;
        let engine = ClientEngine::new(ClientConfig {
            join_token: command.token.clone(),
            client_name: command.name.clone(),
            reconnect: ReconnectPolicy::default(),
        });
        let mut session = Self {
            link,
            transport,
            engine,
            desk: MissionDesk::new(&command.name, now_ms()),
            policy: command.policy,
            started: Instant::now(),
            every_status: true,
            last_shown: None,
            last_telemetry: None,
        };
        let actions = session.engine.handle(TransportEvent::Connected, 0);
        session.perform(actions).await?;
        let deadline = Instant::now() + RECEIPT_TIMEOUT;
        while session.engine.admission().is_none() {
            session.step(deadline).await?;
        }
        emit(&json!({"event": "admitted", "as": command.name}));
        // A command resolves against the lane's state, so the first status
        // comes first; a host without a lane publishes none and its refusal
        // says so.
        let first = Instant::now() + FIRST_STATUS;
        while session.desk.status().is_none() && Instant::now() < first {
            match tokio::time::timeout_at(first, session.transport.recv()).await {
                Ok(Some(event)) => {
                    session.on_transport(event).await?;
                }
                Ok(None) => return Err(OperatorError::Host("the link closed".to_owned())),
                Err(_) => break,
            }
        }
        Ok(session)
    }

    /// Resolves and sends `command`. Returns the request id, or `None`
    /// when the desk refused it, which it reports.
    async fn submit(&mut self, command: &MissionCommand) -> Result<Option<String>, OperatorError> {
        let sent = self
            .desk
            .resolve(command, now_ms())
            .and_then(|action| self.desk.send(&self.engine, action));
        match sent {
            Ok((request_id, actions)) => {
                self.perform(actions).await?;
                Ok(Some(request_id))
            }
            Err(refusal) => {
                emit(&json!({"event": "refused", "by": "desk", "reason": refusal.to_string()}));
                Ok(None)
            }
        }
    }

    /// Sends `command` and waits for its receipt.
    async fn ask(
        &mut self,
        command: &MissionCommand,
    ) -> Result<Option<FiledReceipt>, OperatorError> {
        let Some(request_id) = self.submit(command).await? else {
            return Ok(None);
        };
        let deadline = Instant::now() + RECEIPT_TIMEOUT;
        loop {
            if let Some(receipt) = self.desk.receipt().filter(|r| r.request_id == request_id) {
                return Ok(Some(receipt.clone()));
            }
            self.step(deadline).await?;
        }
    }

    async fn watch(&mut self, deadline: Instant) -> Result<bool, OperatorError> {
        loop {
            if Instant::now() >= deadline {
                return Ok(false);
            }
            if let Some(ModuleEvent::MissionStatus(_)) = self.step(deadline).await?
                && let Some(done) = terminal(self.desk.state())
            {
                return Ok(done);
            }
        }
    }

    async fn run(
        &mut self,
        objective: SurveyFile,
        deadline: Instant,
    ) -> Result<bool, OperatorError> {
        let steps = [
            MissionCommand::Propose { objective },
            MissionCommand::Approve {
                digest: None,
                policy: self.policy,
            },
            MissionCommand::Start { digest: None },
        ];
        for command in &steps {
            if !self.ask(command).await?.is_some_and(|r| r.accepted()) {
                return Ok(false);
            }
            if matches!(command, MissionCommand::Approve { .. }) {
                self.await_state(wire::MissionLaneState::Admitted, deadline)
                    .await?;
            }
        }
        self.watch(deadline).await
    }

    /// Waits until the lane's status reports `state`, so a start names the
    /// proposal the lane admitted.
    async fn await_state(
        &mut self,
        state: wire::MissionLaneState,
        deadline: Instant,
    ) -> Result<(), OperatorError> {
        while self.desk.state() != state {
            self.step(deadline).await?;
        }
        Ok(())
    }

    /// Waits for the next transport event and performs what it asks.
    /// Returns the module event it produced, if any.
    async fn step(&mut self, deadline: Instant) -> Result<Option<ModuleEvent>, OperatorError> {
        let event = tokio::time::timeout_at(deadline, self.transport.recv())
            .await
            .map_err(|_| OperatorError::Host("timed out".to_owned()))?
            .ok_or_else(|| OperatorError::Host("the link closed".to_owned()))?;
        self.on_transport(event).await
    }

    async fn on_transport(
        &mut self,
        event: TransportEvent,
    ) -> Result<Option<ModuleEvent>, OperatorError> {
        let now = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let actions = self.engine.handle(event, now);
        self.perform(actions).await
    }

    async fn perform(
        &mut self,
        actions: Vec<ClientAction>,
    ) -> Result<Option<ModuleEvent>, OperatorError> {
        let mut produced = None;
        for action in actions {
            match action {
                ClientAction::SendBootstrap(bytes) => self
                    .link
                    .send_bootstrap(&bytes)
                    .await
                    .map_err(OperatorError::Link)?,
                ClientAction::Emit(ModuleEvent::Telemetry(sample)) => self.telemetry(&sample),
                ClientAction::Emit(ModuleEvent::MissionStatus(status)) => {
                    self.desk.on_status(status.clone());
                    self.show_status();
                    produced = Some(ModuleEvent::MissionStatus(status));
                }
                ClientAction::Emit(ModuleEvent::MissionReceipt(receipt)) => {
                    self.on_receipt(*receipt)
                }
                ClientAction::Stop(fault) => return Err(OperatorError::Host(fault.to_string())),
                ClientAction::ScheduleReconnect { .. } => {
                    return Err(OperatorError::Host("the connection dropped".to_owned()));
                }
                ClientAction::SendDatagram(_) | ClientAction::Emit(_) => {}
            }
        }
        Ok(produced)
    }

    fn on_receipt(&mut self, receipt: wire::MissionReceipt) {
        let proposal = receipt.proposal.as_ref().map(ProposalView::of);
        let Some(filed) = self.desk.on_receipt(receipt) else {
            return;
        };
        let mut line = json!({"event": "receipt"});
        merge(&mut line, &ReceiptView::of(filed));
        if let Some(proposal) = proposal.filter(|_| filed.accepted()) {
            line["proposal"] = json!(proposal);
        }
        emit(&line);
    }

    /// Writes the lane's status: every one, or in agent mode only a change
    /// of what the lane does, and at least every ten seconds.
    fn show_status(&mut self) {
        let Some(view) = self.desk.view(true, &self.policy) else {
            return;
        };
        let mut line = json!({"event": "status"});
        merge(&mut line, &view);
        let mut gist = line.clone();
        gist["sequence"] = Value::Null;
        if !self.every_status
            && self
                .last_shown
                .as_ref()
                .is_some_and(|(shown, at)| *shown == gist && at.elapsed() < Duration::from_secs(10))
        {
            return;
        }
        self.last_shown = Some((gist, Instant::now()));
        emit(&line);
    }

    /// With `PILOTAGE_OPERATOR_TELEMETRY=1`, one line a second with the
    /// vehicle's position, height and ground speed.
    fn telemetry(&mut self, sample: &wire::TelemetrySample) {
        let Some(avionics) = sample.avionics.as_ref() else {
            return;
        };
        let Some(fix) = avionics.geodetic.as_ref() else {
            return;
        };
        if std::env::var_os("PILOTAGE_OPERATOR_TELEMETRY").is_none()
            || self
                .last_telemetry
                .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.last_telemetry = Some(Instant::now());
        let speed = avionics.vel_n_mps.hypot(avionics.vel_e_mps);
        emit(
            &json!({"event": "telemetry", "lat": fix.latitude_deg, "lon": fix.longitude_deg, "height_m": -avionics.pos_d_m, "speed_mps": speed}),
        );
    }
}

/// `Some(ended well)` for a lane that ended.
fn terminal(state: wire::MissionLaneState) -> Option<bool> {
    match state {
        wire::MissionLaneState::Complete => Some(true),
        wire::MissionLaneState::Failed | wire::MissionLaneState::Aborted => Some(false),
        _ => None,
    }
}

/// Adds the fields of `value` to the object `line`.
fn merge(line: &mut Value, value: &impl serde::Serialize) {
    if let (Some(line), Ok(Value::Object(fields))) =
        (line.as_object_mut(), serde_json::to_value(value))
    {
        line.extend(fields);
    }
}

fn emit(value: &Value) {
    writeln!(std::io::stdout().lock(), "{value}").ok();
}
