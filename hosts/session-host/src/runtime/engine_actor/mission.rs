//! Mission traffic through the engine actor (ADR-0052): client intents go
//! to the host's mission lane; the lane's receipts and status go back to
//! clients on the reliable session-events stream. The session engine does
//! not decide mission content, so this traffic bypasses it.

use pilotage_adapter_api::VehicleAdapter;
use pilotage_protocol::wire;
use pilotage_session::ClientKey;
use tokio::sync::mpsc::error::TrySendError;
use tracing::warn;

use super::EngineActor;
use super::drops::MessageClass;
use crate::runtime::connection::ToConnection;
use crate::runtime::mission_lane::LaneInbound;
use crate::runtime::wire_codec::encode_mission_payload;

impl<A: VehicleAdapter> EngineActor<A> {
    pub(super) fn on_mission_intent(
        &mut self,
        client: ClientKey,
        intent: Box<wire::MissionIntent>,
    ) {
        let request_id = intent.request_id.clone();
        let Some(lane) = self.mission_lane.as_ref() else {
            self.refuse_mission(client, request_id, "this host runs no mission lane");
            return;
        };
        match lane.try_send(LaneInbound::Intent { client, intent }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.refuse_mission(
                    client,
                    request_id,
                    "the mission lane is busy; send the intent again",
                );
            }
            Err(TrySendError::Closed(_)) => {
                self.mission_lane = None;
                self.refuse_mission(client, request_id, "the mission lane has stopped");
            }
        }
    }

    pub(super) fn publish_mission(
        &mut self,
        to: Option<ClientKey>,
        payload: wire::envelope::Payload,
    ) {
        let message = ToConnection::AuthorityMessage(encode_mission_payload(payload));
        match to {
            None => self.broadcast(message, MessageClass::AuthorityBroadcast),
            Some(client) => {
                if let Some(sender) = self.clients.sender(client) {
                    if sender.try_send(message).is_err() {
                        warn!(
                            client = client.as_u64(),
                            "mission receipt not queued for its client"
                        );
                    }
                }
            }
        }
    }

    fn refuse_mission(&mut self, client: ClientKey, request_id: String, reason: &str) {
        let receipt = wire::MissionReceipt {
            request_id,
            outcome: wire::MissionReceiptOutcome::Refused as i32,
            reason: reason.to_owned(),
            proposal: None,
            lane: String::new(),
        };
        self.publish_mission(
            Some(client),
            wire::envelope::Payload::MissionReceipt(receipt),
        );
    }
}
