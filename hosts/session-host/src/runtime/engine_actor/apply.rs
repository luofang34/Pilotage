//! Adapter delivery for gated control frames: exactly-once execution of
//! correlated discrete actions and the per-action result echo (CTRL-01).

use pilotage_adapter_api::{Disposition, RejectReason};
use pilotage_protocol::{FrameRejected, FrameRejectionReason};

use super::*;

impl<A: VehicleAdapter> EngineActor<A> {
    /// Applies a gated frame to the adapter with exactly-once semantics for
    /// its correlated actions (CTRL-01): retransmitted actions the adapter
    /// already answered are stripped and their recorded results replayed;
    /// every action the adapter processes gets its explicit outcome back on
    /// the reliable session stream; and a correlated action the adapter did
    /// NOT answer — an early rejection (link loss engaged, measurement
    /// unavailable, reset in progress, unsupported scope) returns before
    /// per-action disposal — is answered HERE with the rejection's reason,
    /// so a press is never silently dropped and a valid command always
    /// yields exactly one correlated result. A rejected intent-only frame
    /// also returns a reliable frame-level notice, transition-deduplicated so
    /// a steady refusal cannot flood the session stream.
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
        self.report_adapter_rejection(client, &frame, &outcome.disposition);
        let mut answered = vec![false; frame.actions.len()];
        for result in outcome.action_results {
            // Actions cannot repeat within a gated frame, so the action
            // itself keys its correlation id.
            let index = frame
                .actions
                .iter()
                .position(|action| *action == result.action);
            if let Some(index) = index {
                answered[index] = true;
            }
            let action_id = index
                .and_then(|index| frame.action_ids.get(index))
                .copied()
                .unwrap_or(0);
            self.answer_action(
                client,
                &frame,
                result.action,
                result.accepted,
                result.detail,
                action_id,
            );
        }
        // The result guarantee: any correlated action the adapter left
        // unanswered is rejected with the frame-level reason.
        let detail = match &outcome.disposition {
            Disposition::Rejected(reason) => format!("adapter rejected the frame: {reason:?}"),
            _ => "the adapter returned no result for this action".to_owned(),
        };
        for (index, action) in frame.actions.clone().iter().enumerate() {
            let action_id = frame.action_ids.get(index).copied().unwrap_or(0);
            if answered[index] || action_id == 0 {
                continue;
            }
            self.answer_action(client, &frame, *action, false, detail.clone(), action_id);
        }
    }

    fn report_adapter_rejection(
        &mut self,
        client: ClientKey,
        frame: &ScopedControlFrame,
        disposition: &Disposition,
    ) {
        let Disposition::Rejected(reason) = disposition else {
            self.adapter_rejection_dedup
                .observe_enacted(client, frame.vehicle, &frame.scope);
            return;
        };
        let reason = adapter_rejection_reason(reason);
        if !self.adapter_rejection_dedup.should_notify(
            client,
            frame.vehicle,
            &frame.scope,
            frame.generation,
            reason,
        ) {
            return;
        }
        let rejection = FrameRejected {
            vehicle: frame.vehicle,
            scope: frame.scope.clone(),
            sequence: frame.sequence,
            reason,
            current_generation: frame.generation,
        };
        let envelope = pilotage_session::OutboundMessage::FrameRejected(rejection);
        let message = to_connection_message(&envelope);
        self.send_to(client, message, MessageClass::Unicast);
    }

    /// Sends one correlated result on the reliable stream and records it
    /// for replay dedup.
    fn answer_action(
        &mut self,
        client: ClientKey,
        frame: &ScopedControlFrame,
        action: pilotage_protocol::ControlAction,
        accepted: bool,
        detail: String,
        action_id: u32,
    ) {
        let full = pilotage_protocol::ControlActionResult {
            vehicle: frame.vehicle,
            scope: frame.scope.clone(),
            generation: frame.generation,
            sequence: frame.sequence,
            action,
            accepted,
            detail,
            action_id,
        };
        self.action_dedup.record(client, frame, full.clone());
        let envelope = pilotage_session::OutboundMessage::ControlActionResult(full);
        let message = to_connection_message(&envelope);
        self.send_to(client, message, MessageClass::Unicast);
    }
}

/// Maps an adapter-boundary refusal to its wire reason. Exhaustive on
/// purpose: a new [`RejectReason`] variant must decide its wire mapping here
/// rather than silently degrading to the generic reason. `Fenced` and
/// `LinkLossEngaged` deliberately stay generic — the wire's stale-generation
/// and no-holder reasons make the client drop its grant and reacquire, a
/// recovery that must only ever be driven by the SESSION's authority state,
/// never by adapter-internal bookkeeping downstream of an admitted frame.
fn adapter_rejection_reason(reason: &RejectReason) -> FrameRejectionReason {
    match reason {
        RejectReason::UnknownScope => FrameRejectionReason::UnknownScope,
        RejectReason::UplinkIdle => FrameRejectionReason::UplinkIdle,
        RejectReason::UnknownAxis
        | RejectReason::UnknownVehicle
        | RejectReason::Fenced
        | RejectReason::MeasurementUnavailable
        | RejectReason::LinkLossEngaged
        | RejectReason::ResetInProgress
        | RejectReason::Other(_) => FrameRejectionReason::AdapterRejected,
    }
}
