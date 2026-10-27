//! An operator connection to the engine actor, as a client's connection
//! task drives it.

use pilotage_protocol::{
    ClientHello, PrincipalId, SESSION_PROTOCOL_VERSION, ScopeId, ScopeTransferAccept,
    ScopeTransferOffer, ScopeTransferRequest, wire,
};
use pilotage_session::{ClientKey, DomainEnvelope};
use tokio::sync::mpsc;

use super::{PHASE, Rig};
use crate::runtime::HOST_VEHICLE;
use crate::runtime::connection::ToConnection;
use crate::runtime::engine_actor::ToEngine;
use crate::runtime::registry::OUTBOUND_QUEUE_CAPACITY;

/// An operator connection to the engine actor.
pub(super) struct Operator {
    client: ClientKey,
    engine_tx: mpsc::Sender<ToEngine>,
    outbound: mpsc::Receiver<ToConnection>,
    principal: u64,
}

impl Operator {
    pub(super) async fn connect(rig: &Rig, key: u64) -> Self {
        let (sender, outbound) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
        let client = ClientKey::new(key);
        rig.engine_tx
            .send(ToEngine::ClientConnected { client, sender })
            .await
            .expect("the actor runs");
        let mut operator = Self {
            client,
            engine_tx: rig.engine_tx.clone(),
            outbound,
            principal: 0,
        };
        operator
            .send(DomainEnvelope::Hello(ClientHello {
                protocol_version: SESSION_PROTOCOL_VERSION,
                client_name: "operator".to_owned(),
                join_token: Vec::new(),
            }))
            .await;
        operator.principal = operator
            .await_payload(|payload| match payload {
                wire::envelope::Payload::ServerWelcome(welcome) => {
                    welcome.principal.map(|p| p.value)
                }
                _ => None,
            })
            .await;
        operator
    }

    async fn send(&self, message: DomainEnvelope) {
        self.engine_tx
            .send(ToEngine::ClientMessage {
                client: self.client,
                message: Box::new(message),
                now: pilotage_timing::MonoTimestamp::from_nanos(0),
            })
            .await
            .expect("the actor runs");
    }

    /// The first reliable or authority payload `pick` takes.
    async fn await_payload<T>(&mut self, pick: impl Fn(wire::envelope::Payload) -> Option<T>) -> T {
        tokio::time::timeout(PHASE, async {
            loop {
                let bytes = match self.outbound.recv().await.expect("the actor runs") {
                    ToConnection::BootstrapMessage { bytes, .. }
                    | ToConnection::AuthorityMessage(bytes) => bytes,
                    _ => continue,
                };
                let Ok((envelope, _)) = pilotage_protocol::decode_envelope_length_delimited(&bytes)
                else {
                    continue;
                };
                if let Some(found) = envelope.payload.and_then(&pick) {
                    return found;
                }
            }
        })
        .await
        .expect("the awaited payload arrives")
    }

    /// Takes the motion scope from the mission by transfer; returns the
    /// mission principal that offered it.
    pub(super) async fn take_control(&mut self) -> u64 {
        let scope = ScopeId::new("vehicle.motion");
        self.send(DomainEnvelope::TransferRequest(ScopeTransferRequest {
            vehicle: HOST_VEHICLE,
            scope: scope.clone(),
        }))
        .await;
        let me = self.principal;
        let mission = self
            .await_payload(|payload| match payload {
                wire::envelope::Payload::AuthorityEvent(event) => match event.event {
                    Some(wire::authority_event::Event::ScopeTransferOffered(offered))
                        if offered.to_principal.as_ref().is_some_and(|p| p.value == me) =>
                    {
                        offered.from_principal.map(|p| p.value)
                    }
                    _ => None,
                },
                _ => None,
            })
            .await;
        self.send(DomainEnvelope::TransferAccept(ScopeTransferAccept {
            vehicle: HOST_VEHICLE,
            scope,
        }))
        .await;
        mission
    }

    pub(super) async fn hand_back(&self, mission: u64) {
        self.send(DomainEnvelope::TransferOffer(ScopeTransferOffer {
            vehicle: HOST_VEHICLE,
            scope: ScopeId::new("vehicle.motion"),
            to: PrincipalId::new(mission),
        }))
        .await;
    }

    /// The connection closes without a hand-back.
    pub(super) async fn disconnect(self) {
        self.send(DomainEnvelope::Disconnect).await;
    }
}
