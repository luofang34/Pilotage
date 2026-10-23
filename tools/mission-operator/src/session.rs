//! One operator session with a host: admission, intents, receipts and the
//! lane's status, each written as one JSON line.

use std::io::Write as _;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pilotage_client_link::Link;
use pilotage_client_session::{
    ClientAction, ClientConfig, ClientEngine, ModuleEvent, ReconnectPolicy, TransportEvent,
};
use pilotage_protocol::wire;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::cli::{Action, Command, Envelope, OperatorError, hex};
use crate::objective;

/// How long a receipt may take: a team answers after its members do.
const RECEIPT_TIMEOUT: Duration = Duration::from_secs(60);

struct Session {
    link: Link,
    transport: mpsc::Receiver<TransportEvent>,
    engine: ClientEngine,
    started: Instant,
    requests: u64,
    /// The newest status of the lane.
    last_status: Option<wire::MissionStatus>,
    /// Receipts not yet claimed by the intent they answer.
    receipts: Vec<wire::MissionReceipt>,
}

pub(crate) async fn execute(command: Command) -> Result<bool, OperatorError> {
    let mut session = Session::open(&command.url, command.cert_sha256).await?;
    let deadline = Instant::now() + Duration::from_secs(command.seconds);
    let outcome = match command.action {
        Action::Propose(path) => {
            let objective = objective::read_blocking(&path)?;
            session
                .ask(wire::mission_intent::Action::Propose(objective))
                .await
                .map(|r| accepted(&r))
        }
        Action::Approve(digest, envelope) => session
            .ask(approval(digest, envelope))
            .await
            .map(|r| accepted(&r)),
        Action::Start(digest) => session.ask(start(digest)).await.map(|r| accepted(&r)),
        Action::Hold => session
            .ask(wire::mission_intent::Action::Hold(wire::MissionHold {}))
            .await
            .map(|r| accepted(&r)),
        Action::Resume => session
            .ask(wire::mission_intent::Action::Resume(wire::MissionResume {}))
            .await
            .map(|r| accepted(&r)),
        Action::Abort => session
            .ask(wire::mission_intent::Action::Abort(wire::MissionAbort {}))
            .await
            .map(|r| accepted(&r)),
        Action::Watch => session.watch(deadline).await,
        Action::Run(path, envelope) => session.run(&path, envelope, deadline).await,
    };
    session.link.shutdown().await;
    outcome
}

fn accepted(receipt: &wire::MissionReceipt) -> bool {
    receipt.outcome == wire::MissionReceiptOutcome::Accepted as i32
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn approval(digest: Vec<u8>, envelope: Envelope) -> wire::mission_intent::Action {
    let now = now_ms();
    wire::mission_intent::Action::Approve(wire::MissionApproval {
        approval_id: format!("approval-{now}"),
        proposal_digest: digest,
        approver: Some(wire::RecordOrigin {
            issuer: "local".to_owned(),
            subject: "mission-operator".to_owned(),
            organization: String::new(),
        }),
        envelope: Some(wire::AdaptationEnvelope {
            max_height_m: envelope.max_height_m,
            min_reserve_fraction: envelope.min_reserve,
            expires_unix_ms: now + u64::try_from(envelope.expires.as_millis()).unwrap_or(u64::MAX),
            bound: Vec::new(),
            permit_execute: true,
            permit_revise: envelope.revise,
            permit_allocate: true,
            permit_change_objective: false,
            max_refly_lines: envelope.refly_lines,
        }),
        approved_unix_ms: now,
    })
}

fn start(digest: Vec<u8>) -> wire::mission_intent::Action {
    wire::mission_intent::Action::Start(wire::MissionStart {
        proposal_digest: digest,
    })
}

impl Session {
    async fn open(url: &str, cert_sha256: [u8; 32]) -> Result<Self, OperatorError> {
        let (link, transport) = Link::connect(url, cert_sha256)
            .await
            .map_err(OperatorError::Link)?;
        let engine = ClientEngine::new(ClientConfig {
            join_token: None,
            client_name: "mission-operator".to_owned(),
            reconnect: ReconnectPolicy::default(),
        });
        let mut session = Self {
            link,
            transport,
            engine,
            started: Instant::now(),
            requests: 0,
            last_status: None,
            receipts: Vec::new(),
        };
        let actions = session.engine.handle(TransportEvent::Connected, 0);
        session.perform(actions).await?;
        let deadline = Instant::now() + RECEIPT_TIMEOUT;
        while session.engine.admission().is_none() {
            session.step(deadline).await?;
        }
        Ok(session)
    }

    /// Sends one intent and waits for its receipt.
    async fn ask(
        &mut self,
        action: wire::mission_intent::Action,
    ) -> Result<wire::MissionReceipt, OperatorError> {
        self.requests = self.requests.wrapping_add(1);
        let request_id = format!("operator-{}-{}", now_ms(), self.requests);
        let actions = self.engine.mission_intent(wire::MissionIntent {
            request_id: request_id.clone(),
            action: Some(action),
        });
        self.perform(actions).await?;
        let deadline = Instant::now() + RECEIPT_TIMEOUT;
        loop {
            if let Some(index) = self
                .receipts
                .iter()
                .position(|r| r.request_id == request_id)
            {
                let receipt = self.receipts.swap_remove(index);
                emit(&receipt_json(&receipt));
                return Ok(receipt);
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
                && let Some(done) = self.last_status.as_ref().and_then(|s| terminal(s.state))
            {
                return Ok(done);
            }
        }
    }

    async fn run(
        &mut self,
        path: &std::path::Path,
        envelope: Envelope,
        deadline: Instant,
    ) -> Result<bool, OperatorError> {
        let objective = objective::read_blocking(path)?;
        let receipt = self
            .ask(wire::mission_intent::Action::Propose(objective))
            .await?;
        let Some(digest) = receipt
            .proposal
            .as_ref()
            .filter(|_| accepted(&receipt))
            .map(|p| p.digest.clone())
        else {
            return Ok(false);
        };
        if !accepted(&self.ask(approval(digest.clone(), envelope)).await?) {
            return Ok(false);
        }
        if !accepted(&self.ask(start(digest)).await?) {
            return Ok(false);
        }
        self.watch(deadline).await
    }

    /// Waits for the next transport event and performs what it asks.
    /// Returns the module event it produced, if any.
    async fn step(&mut self, deadline: Instant) -> Result<Option<ModuleEvent>, OperatorError> {
        let event = tokio::time::timeout_at(deadline, self.transport.recv())
            .await
            .map_err(|_| OperatorError::Host("timed out".to_owned()))?
            .ok_or_else(|| OperatorError::Host("the link closed".to_owned()))?;
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
                ClientAction::Emit(ModuleEvent::MissionStatus(status)) => {
                    emit(&status_json(&status));
                    self.last_status = Some((*status).clone());
                    produced = Some(ModuleEvent::MissionStatus(status));
                }
                ClientAction::Emit(ModuleEvent::MissionReceipt(receipt)) => {
                    self.receipts.push(*receipt)
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
}

/// `Some(ended well)` for a lane that ended.
fn terminal(state: i32) -> Option<bool> {
    match wire::MissionLaneState::try_from(state).ok()? {
        wire::MissionLaneState::Complete => Some(true),
        wire::MissionLaneState::Failed | wire::MissionLaneState::Aborted => Some(false),
        _ => None,
    }
}

fn state_name(state: i32) -> String {
    wire::MissionLaneState::try_from(state)
        .map_or_else(|_| state.to_string(), |s| format!("{s:?}").to_lowercase())
}

fn task_state_name(state: i32) -> String {
    wire::MissionTaskState::try_from(state)
        .map_or_else(|_| state.to_string(), |s| format!("{s:?}").to_lowercase())
}

fn status_json(status: &wire::MissionStatus) -> Value {
    let tasks: Vec<Value> = status
        .tasks
        .iter()
        .map(|t| {
            let e = t.evidence.clone().unwrap_or_default();
            json!({"task": t.task_id, "state": task_state_name(t.state), "frames": e.frames, "assessed": e.assessed, "usable": e.usable, "reflown": t.reflown})
        })
        .collect();
    let members: Vec<Value> = status
        .members
        .iter()
        .map(|m| json!({"lane": m.lane, "state": state_name(m.state), "active": m.active_task, "simulated": m.simulated, "detail": m.detail}))
        .collect();
    json!({"status": status.lane, "state": state_name(status.state), "sequence": status.sequence, "simulated": status.simulated, "active": status.active_task, "detail": status.detail, "tasks": tasks, "members": members})
}

fn receipt_json(receipt: &wire::MissionReceipt) -> Value {
    let proposal = receipt.proposal.as_ref().map(|p| {
        let e = p.estimates.unwrap_or_default();
        json!({"digest": hex(&p.digest), "tasks": p.tasks.len(), "members": p.members.len(), "lines": e.lines, "path_length_m": e.path_length_m.round(), "duration_s": e.duration_s.round(), "line_spacing_m": e.line_spacing_m})
    });
    json!({"receipt": receipt.request_id, "lane": receipt.lane, "accepted": accepted(receipt), "reason": receipt.reason, "proposal": proposal})
}

fn emit(value: &Value) {
    writeln!(std::io::stdout().lock(), "{value}").ok();
}
