//! One member host's link: the coordinator is an ordinary client of it. It
//! sends the team's intents and reports the member lane's receipts and
//! status. A lost link reconnects with the client session's policy.

use std::time::Duration;

use pilotage_client_link::Link;
use pilotage_client_session::{
    ClientAction, ClientConfig, ClientEngine, ModuleEvent, ReconnectPolicy, TransportEvent,
};
use pilotage_protocol::wire;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{info, warn};

use super::config::MemberHost;

/// What a member link reports.
#[derive(Debug)]
pub(super) enum MemberEvent {
    Receipt {
        member: String,
        receipt: Box<wire::MissionReceipt>,
    },
    Status {
        status: Box<wire::MissionStatus>,
    },
    Lost {
        member: String,
    },
}

/// The wait before the first reconnection attempt.
const RETRY: Duration = Duration::from_secs(2);

pub(super) fn spawn(
    host: MemberHost,
    intents: mpsc::Receiver<wire::MissionIntent>,
    events: mpsc::Sender<MemberEvent>,
) {
    tokio::spawn(async move {
        run(host, intents, events).await;
        tracing::debug!(task = "member-link", "task exited");
    });
}

async fn run(
    host: MemberHost,
    mut intents: mpsc::Receiver<wire::MissionIntent>,
    events: mpsc::Sender<MemberEvent>,
) {
    let started = Instant::now();
    loop {
        match Link::connect(&host.url, host.cert_sha256).await {
            Ok((link, transport)) => {
                info!(member = %host.name, url = %host.url, "member link up");
                let ended = drive(&host, link, transport, &mut intents, &events, started).await;
                if ended {
                    return;
                }
            }
            Err(error) => warn!(member = %host.name, %error, "member link failed"),
        }
        if events
            .send(MemberEvent::Lost {
                member: host.name.clone(),
            })
            .await
            .is_err()
        {
            return;
        }
        tokio::time::sleep(RETRY).await;
    }
}

/// Runs one connection. Returns `true` when the coordinator is gone.
async fn drive(
    host: &MemberHost,
    mut link: Link,
    mut transport: mpsc::Receiver<TransportEvent>,
    intents: &mut mpsc::Receiver<wire::MissionIntent>,
    events: &mpsc::Sender<MemberEvent>,
    started: Instant,
) -> bool {
    let mut engine = ClientEngine::new(ClientConfig {
        join_token: host.token.clone(),
        client_name: "pilotage-coordinator".to_owned(),
        reconnect: ReconnectPolicy::default(),
    });
    let now = || u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut actions = engine.handle(TransportEvent::Connected, now());
    // Intents wait for the member host's admission.
    let mut waiting: Vec<wire::MissionIntent> = Vec::new();
    loop {
        let admitted = match perform(host, &mut link, std::mem::take(&mut actions), events).await {
            Flow::Continue(admitted) => admitted,
            Flow::Reconnect => {
                link.shutdown().await;
                return false;
            }
            Flow::Exit => return true,
        };
        if admitted {
            for intent in waiting.drain(..) {
                actions.extend(engine.mission_intent(intent));
            }
            if !actions.is_empty() {
                continue;
            }
        }
        tokio::select! {
            event = transport.recv() => match event {
                Some(event) => actions = engine.handle(event, now()),
                None => {
                    link.shutdown().await;
                    return false;
                }
            },
            intent = intents.recv() => match intent {
                Some(intent) => {
                    actions = engine.mission_intent(intent.clone());
                    if actions.is_empty() {
                        waiting.push(intent);
                    }
                }
                None => {
                    link.shutdown().await;
                    return true;
                }
            },
        }
    }
}

/// What one batch of client actions leaves the link to do.
enum Flow {
    /// Keep going; whether the batch admitted the session.
    Continue(bool),
    /// The connection is gone; connect again.
    Reconnect,
    /// The coordinator is gone.
    Exit,
}

async fn perform(
    host: &MemberHost,
    link: &mut Link,
    actions: Vec<ClientAction>,
    events: &mpsc::Sender<MemberEvent>,
) -> Flow {
    let mut admitted = false;
    for action in actions {
        let event = match action {
            ClientAction::Emit(ModuleEvent::Admitted(_)) => {
                admitted = true;
                None
            }
            ClientAction::SendBootstrap(bytes) => {
                if link.send_bootstrap(&bytes).await.is_err() {
                    return Flow::Reconnect;
                }
                None
            }
            ClientAction::Emit(ModuleEvent::MissionReceipt(receipt)) => {
                Some(MemberEvent::Receipt {
                    member: host.name.clone(),
                    receipt,
                })
            }
            ClientAction::Emit(ModuleEvent::MissionStatus(status)) => {
                Some(MemberEvent::Status { status })
            }
            ClientAction::ScheduleReconnect { .. } | ClientAction::Stop(_) => {
                return Flow::Reconnect;
            }
            ClientAction::SendDatagram(_) | ClientAction::Emit(_) => None,
        };
        if let Some(event) = event
            && events.send(event).await.is_err()
        {
            return Flow::Exit;
        }
    }
    Flow::Continue(admitted)
}
