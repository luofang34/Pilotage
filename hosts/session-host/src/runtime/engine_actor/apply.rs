//! Adapter delivery for gated control frames: exactly-once execution of
//! correlated discrete actions and the per-action result echo (CTRL-01).

use super::*;

impl<A: VehicleAdapter> EngineActor<A> {
    /// Applies a gated frame to the adapter with exactly-once semantics for
    /// its correlated actions (CTRL-01): retransmitted actions the adapter
    /// already answered are stripped and their recorded results replayed,
    /// and every action the adapter processes gets its explicit outcome back
    /// on the reliable session stream — a press is never silently dropped.
    pub(super) fn apply_to_adapter(&mut self, client: ClientKey, mut frame: ScopedControlFrame) {
        for replay in self.action_dedup.strip_answered(client, &mut frame) {
            let envelope = pilotage_session::OutboundMessage::ControlActionResult(replay);
            let message = to_connection_message(&envelope);
            self.send_to(client, message, MessageClass::Unicast);
        }
        // Stripping can drain the frame entirely (a pure retransmission of
        // answered presses); an empty command has nothing for the adapter.
        if frame.intent.is_none() && frame.actions.is_empty() && !frame.carries_payload() {
            return;
        }
        let apply_start = Instant::now();
        let outcome = self.adapter.apply_control(&frame);
        self.record_stage(Stage::Apply, apply_start.elapsed());
        debug!(?outcome, "control frame applied to adapter");
        for result in outcome.action_results {
            // Actions cannot repeat within a gated frame, so the action
            // itself keys its correlation id.
            let action_id = frame
                .actions
                .iter()
                .position(|action| *action == result.action)
                .and_then(|index| frame.action_ids.get(index))
                .copied()
                .unwrap_or(0);
            let full = pilotage_protocol::ControlActionResult {
                vehicle: frame.vehicle,
                scope: frame.scope.clone(),
                generation: frame.generation,
                sequence: frame.sequence,
                action: result.action,
                accepted: result.accepted,
                detail: result.detail,
                action_id,
            };
            self.action_dedup.record(client, &frame, full.clone());
            let envelope = pilotage_session::OutboundMessage::ControlActionResult(full);
            let message = to_connection_message(&envelope);
            self.send_to(client, message, MessageClass::Unicast);
        }
    }
}
