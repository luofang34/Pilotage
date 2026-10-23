//! Mission authority through the production WebTransport host: a client
//! admitted with only the join secret proposes but cannot approve, and the
//! approval the host accepts names the session that sent it.

#![cfg(test)]

use std::time::Duration;

use pilotage_client_link::Link;
use pilotage_client_session::mission::{
    ApprovalPolicy, FiledReceipt, MissionCommand, MissionDesk, SurveyFile,
};
use pilotage_client_session::{
    ClientAction, ClientConfig, ClientEngine, ModuleEvent, ReconnectPolicy, TransportEvent,
};
use pilotage_protocol::{JoinToken, wire};
use pilotage_session_host::cli::AdapterKind;
use pilotage_session_host::runtime::{self, NetworkOptions, RuntimeOptions};
use tokio::sync::mpsc;

const SURVEY: &str = r#"{"title": "approver test",
    "area": [[40.3460, -74.6610], [40.3460, -74.6584], [40.3480, -74.6584], [40.3480, -74.6610]],
    "launch": [40.3459, -74.6611, 30.0], "height_m": 60, "cruise_mps": 10}"#;

struct Client {
    link: Link,
    transport: mpsc::Receiver<TransportEvent>,
    engine: ClientEngine,
    desk: MissionDesk,
}

impl Client {
    async fn connect(port: u16, cert: [u8; 32], name: &str, token: JoinToken) -> Self {
        let url = format!("https://127.0.0.1:{port}/pilotage");
        let (link, transport) = Link::connect(&url, cert).await.unwrap();
        let mut client = Self {
            link,
            transport,
            engine: ClientEngine::new(ClientConfig {
                join_token: Some(token),
                client_name: name.to_owned(),
                reconnect: ReconnectPolicy::default(),
            }),
            desk: MissionDesk::new(name, 1),
        };
        let actions = client.engine.handle(TransportEvent::Connected, 0);
        client.perform(actions).await;
        while client.engine.admission().is_none() {
            client.step().await;
        }
        client
    }

    async fn step(&mut self) {
        let event = tokio::time::timeout(Duration::from_secs(10), self.transport.recv())
            .await
            .unwrap()
            .unwrap();
        let actions = self.engine.handle(event, 0);
        self.perform(actions).await;
    }

    async fn perform(&mut self, actions: Vec<ClientAction>) {
        for action in actions {
            match action {
                ClientAction::SendBootstrap(bytes) => {
                    self.link.send_bootstrap(&bytes).await.unwrap()
                }
                ClientAction::Emit(ModuleEvent::MissionReceipt(receipt)) => {
                    self.desk.on_receipt(*receipt);
                }
                ClientAction::Emit(ModuleEvent::MissionStatus(status)) => {
                    self.desk.on_status(status)
                }
                _ => {}
            }
        }
    }

    async fn ask(&mut self, command: MissionCommand) -> FiledReceipt {
        let action = self.desk.resolve(&command, 1_800_000_000_000).unwrap();
        let (request_id, actions) = self.desk.send(&self.engine, action).unwrap();
        self.perform(actions).await;
        loop {
            if let Some(receipt) = self.desk.receipt().filter(|r| r.request_id == request_id) {
                return receipt.clone();
            }
            self.step().await;
        }
    }
}

#[test]
fn only_the_approver_secret_approves_a_proposal() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let join = JoinToken::from_hex(&"ab".repeat(32)).unwrap();
        let approver = JoinToken::from_hex(&"cd".repeat(32)).unwrap();
        let host = runtime::start_with_options(
            0,
            AdapterKind::Reference,
            RuntimeOptions {
                network: NetworkOptions {
                    join_token: Some(join.clone()),
                    approver_token: Some(approver.clone()),
                    ..NetworkOptions::default()
                },
                ..RuntimeOptions::default()
            },
        )
        .await
        .unwrap();
        let cert: [u8; 32] = (0..32)
            .map(|i| u8::from_str_radix(&host.cert_hash_hex[2 * i..2 * i + 2], 16).unwrap())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let port = host.local_addr.port();
        let mut agent = Client::connect(port, cert, "mission-agent", join).await;
        let mut person = Client::connect(port, cert, "sokoly-app", approver).await;

        let objective = SurveyFile::parse(SURVEY).unwrap();
        let proposed = agent.ask(MissionCommand::Propose { objective }).await;
        assert!(proposed.accepted(), "{}", proposed.reason);
        let digest = agent.desk.current_proposal().unwrap().digest.clone();

        let refused = agent
            .ask(MissionCommand::Approve {
                digest: None,
                policy: ApprovalPolicy::default(),
            })
            .await;
        assert!(!refused.accepted());
        assert!(
            refused.reason.contains("mission-agent may not approve"),
            "{}",
            refused.reason
        );

        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        let approved = person
            .ask(MissionCommand::Approve {
                digest: Some(hex),
                policy: ApprovalPolicy {
                    max_height_m: Some(70.0),
                    ..ApprovalPolicy::default()
                },
            })
            .await;
        assert!(approved.accepted(), "{}", approved.reason);
        while agent.desk.state() != wire::MissionLaneState::Admitted {
            agent.step().await;
        }
        agent.link.shutdown().await;
        person.link.shutdown().await;
        host.shutdown().await;
    });
}
