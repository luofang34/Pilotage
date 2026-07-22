//! The reliable action-command path (CTRL-01): typed discrete actions
//! arrive on the ORDERED session stream carrying their full authority
//! binding — session, vehicle, scope, fencing generation, and announced
//! activation revision — and every command is answered with a
//! `ControlActionResult` on the same stream. A command that fails any
//! binding check is rejected with the reason in the result's detail; only a
//! fully bound command reaches the adapter, so a delayed or replayed press
//! can never fire under authority it no longer holds.

use pilotage_protocol::{
    ControlActionCommand, ControlActionResult, ControlPayload, ScopedControlFrame, SequenceNum,
};
use pilotage_timing::MonoTimestamp;

use crate::action::{CloseReason, SessionAction};
use crate::clients::ScopePair;
use crate::engine::{Actions, SessionEngine};
use crate::message::ClientKey;
use crate::outbound::OutboundMessage;

impl SessionEngine {
    /// Validates and applies one reliable action command.
    pub(super) fn on_action_command(
        &mut self,
        client: ClientKey,
        command: ControlActionCommand,
        now: MonoTimestamp,
        actions: &mut Actions,
    ) {
        let Some(sender) = self.welcomed_principal(client, actions) else {
            return;
        };
        let Some(state) = self.clients.get(client) else {
            return;
        };
        // A command naming a foreign session is forged or broken — close,
        // exactly like a foreign-session profile activation.
        if command.session != state.session {
            actions.push(SessionAction::CloseClient {
                client,
                reason: CloseReason::ProfileSessionMismatch {
                    announced: command.session,
                    expected: state.session,
                },
            });
            return;
        }
        if let Err(detail) = self.action_command_binding(client, sender, &command) {
            actions.send(client, rejected_result(&command, detail));
            return;
        }
        // Deliver as an actions-only typed frame so the adapter boundary,
        // per-action results, and the actor's exactly-once bookkeeping stay
        // on the single existing path.
        let frame = ScopedControlFrame {
            session: command.session,
            vehicle: command.vehicle,
            scope: command.scope.clone(),
            generation: command.generation,
            sequence: SequenceNum::new(0),
            sampled_at: now,
            profile_revision: self
                .clients
                .active_profile(client)
                .map_or(0, |active| active.profile_revision),
            activation_revision: command.activation_revision,
            payload: ControlPayload::default(),
            intent: None,
            actions: vec![command.action],
            action_ids: vec![command.action_id],
        };
        actions.push(SessionAction::ApplyToAdapter { client, frame });
    }

    /// Every binding check between a command and this engine's own records.
    /// Returns the human-readable rejection detail on the first mismatch.
    fn action_command_binding(
        &self,
        client: ClientKey,
        sender: pilotage_protocol::PrincipalId,
        command: &ControlActionCommand,
    ) -> Result<(), &'static str> {
        if command.action_id == 0 {
            return Err("action commands need a nonzero correlation id");
        }
        let pair: ScopePair = (command.vehicle, command.scope.clone());
        if !self.clients.is_registered(&pair) {
            return Err("unknown scope");
        }
        if self.clients.holder_of(&pair) != Some(sender) {
            return Err("sender does not hold the scope");
        }
        if self.clients.generation_of(&pair) != Some(command.generation) {
            return Err("stale generation: authority was re-fenced since this press");
        }
        let bound = self
            .clients
            .active_profile(client)
            .is_some_and(|active| active.activation_revision == command.activation_revision);
        if !bound {
            return Err("activation revision does not match the announced profile");
        }
        let Some(descriptor) = crate::capabilities::scope_capability(
            &self.capabilities,
            command.vehicle,
            &command.scope,
        ) else {
            return Err("unknown scope");
        };
        if crate::command_gate::validate_action(command.action, descriptor).is_err() {
            return Err("action not advertised for this scope");
        }
        Ok(())
    }
}

/// A rejected result echoing the command's binding and correlation id, so
/// the sender learns exactly which press failed and why.
fn rejected_result(command: &ControlActionCommand, detail: &str) -> OutboundMessage {
    OutboundMessage::ControlActionResult(ControlActionResult {
        vehicle: command.vehicle,
        scope: command.scope.clone(),
        generation: command.generation,
        sequence: SequenceNum::new(0),
        action: command.action,
        accepted: false,
        detail: detail.to_owned(),
        action_id: command.action_id,
    })
}
