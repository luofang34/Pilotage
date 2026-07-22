//! The control-frame ingress path (ADR-0009, CTRL-01): staleness, holder
//! fencing, the datagram action refusal, profile binding, the command gate,
//! and acceptance bookkeeping for setpoint frames arriving on the droppable
//! datagram channel.

use pilotage_authority::FrameVerdict;
use pilotage_protocol::{
    FrameRejected, FrameRejectionReason, Generation, PrincipalId, ScopedControlFrame,
};
use pilotage_timing::{Freshness, MonoTimestamp};

use crate::action::SessionAction;
use crate::engine::{Actions, SessionEngine};
use crate::message::ClientKey;

impl SessionEngine {
    /// Staleness-checks, fence-verifies, then forwards or rejects a control
    /// frame (ADR-0009).
    pub(super) fn on_frame(
        &mut self,
        client: ClientKey,
        frame: ScopedControlFrame,
        now: MonoTimestamp,
        actions: &mut Actions,
    ) {
        let Some(sender) = self.welcomed_principal(client, actions) else {
            return;
        };
        if !self.frame_channel_admits(client, &frame, actions) {
            return;
        }
        if self.frame_attribution_rejects(client, &frame, now, actions) {
            return;
        }
        // Fence on the sender's identity, not just on generation. `verify_frame`
        // confirms a holder exists at the current generation, but generations are
        // broadcast to every client, so a non-holder could otherwise forge an
        // in-generation frame. Only the recorded holder may drive the scope. An
        // unregistered scope has no holder and is left to `verify_frame` so the
        // sender still learns the scope is unknown rather than merely unheld.
        let pair = self
            .authority_pair(frame.vehicle, &frame.scope)
            .unwrap_or_else(|| (frame.vehicle, frame.scope.clone()));
        if self.sender_lacks_member_hold(&pair, sender, &frame.scope) {
            let generation = self.frame_generation(&frame);
            actions.push(reject_frame(
                client,
                &frame,
                FrameRejectionReason::NoHolder,
                generation,
            ));
            return;
        }
        match self
            .authority
            .verify_frame(pair.0, &pair.1, frame.generation)
        {
            FrameVerdict::Accepted => self.gate_and_accept(frame, sender, client, now, actions),
            FrameVerdict::RejectedStaleGeneration { current } => {
                actions.push(reject_frame(
                    client,
                    &frame,
                    FrameRejectionReason::StaleGeneration,
                    current,
                ));
            }
            FrameVerdict::RejectedNoHolder => {
                let generation = self.frame_generation(&frame);
                actions.push(reject_frame(
                    client,
                    &frame,
                    FrameRejectionReason::NoHolder,
                    generation,
                ));
            }
            FrameVerdict::RejectedUnknownScope => {
                actions.push(reject_frame(
                    client,
                    &frame,
                    FrameRejectionReason::UnknownScope,
                    Generation::new(0),
                ));
            }
        }
    }

    /// Session attribution, the typed-only production default, and
    /// correlated staleness — everything judged before the frame may touch
    /// holder, liveness, or sequence state. Returns whether the frame was
    /// rejected.
    fn frame_attribution_rejects(
        &mut self,
        client: ClientKey,
        frame: &ScopedControlFrame,
        now: MonoTimestamp,
        actions: &mut Actions,
    ) -> bool {
        // A frame naming a foreign session cannot be attributed to this
        // sender's records (activation binding, sequence state): refused
        // before anything else reads it.
        let session_matches = self
            .clients
            .get(client)
            .is_some_and(|state| state.session == frame.session);
        if !session_matches {
            let generation = self.frame_generation(frame);
            actions.push(reject_frame(
                client,
                frame,
                FrameRejectionReason::SessionMismatch,
                generation,
            ));
            return true;
        }
        // Typed-only is the production default: legacy numeric payloads
        // bypass profile-activation binding and translate edges into
        // uncorrelated actions, so they are admitted only under the
        // explicit SIMULATION compatibility mode.
        if frame.carries_payload() && !self.config.legacy_compatibility {
            let generation = self.frame_generation(frame);
            actions.push(reject_frame(
                client,
                frame,
                FrameRejectionReason::LegacyDisabled,
                generation,
            ));
            return true;
        }
        // The client's sample clock shares no epoch with the host clock, so
        // staleness is judged on CORRELATED age: delay beyond the smallest
        // delta this client has ever shown (offset + minimum path), via
        // `pilotage_timing::estimated_age` — never a raw cross-clock
        // subtraction.
        let age = self.clients.correlated_age(client, now, frame.sampled_at);
        if let Freshness::Stale { .. } = self.staleness.check(age) {
            let generation = self.frame_generation(frame);
            actions.push(reject_frame(
                client,
                frame,
                FrameRejectionReason::TooOld,
                generation,
            ));
            return true;
        }
        false
    }

    /// Whether the registered pair is NOT held by `sender` FOR `member`:
    /// the holder commands the member scope it leased and only that member
    /// — group authority is exclusive across siblings, never a license to
    /// drive them all, so a frame on the unleased sibling is a
    /// non-holder's frame.
    fn sender_lacks_member_hold(
        &self,
        pair: &crate::clients::ScopePair,
        sender: PrincipalId,
        member: &pilotage_protocol::ScopeId,
    ) -> bool {
        self.clients.is_registered(pair)
            && (self.clients.holder_of(pair) != Some(sender)
                || self.clients.held_member(pair) != Some(member))
    }

    /// Typed discrete actions ride ONLY the reliable ordered session
    /// stream (CTRL-01): a datagram-borne edge can be dropped, duplicated
    /// by a retransmitting sender, or reordered past its inverse — a
    /// delayed ARM landing after a DISARM re-arms the vehicle. Legacy
    /// payload edges stay admitted; they are translated (and uncorrelated)
    /// at the single compatibility boundary.
    fn frame_channel_admits(
        &mut self,
        client: ClientKey,
        frame: &ScopedControlFrame,
        actions: &mut Actions,
    ) -> bool {
        if frame.actions.is_empty() || frame.carries_payload() {
            return true;
        }
        let generation = self.frame_generation(frame);
        actions.push(reject_frame(
            client,
            frame,
            FrameRejectionReason::ActionOnDatagram,
            generation,
        ));
        false
    }

    /// Runs a fence-verified frame through the typed-command gate (CTRL-01):
    /// exactly one command representation, capability-validated, with any
    /// legacy payload translated at that single boundary — the adapter only
    /// ever sees typed commands. A gated frame proceeds to acceptance; a
    /// rejected one is echoed to its sender with the typed reason.
    fn gate_and_accept(
        &mut self,
        frame: ScopedControlFrame,
        sender: PrincipalId,
        client: ClientKey,
        now: MonoTimestamp,
        actions: &mut Actions,
    ) {
        // A typed frame binds to profile evidence through its activation
        // revision (INPUT-01): it must equal the sender's announced
        // activation, or the frame cannot be traced to the mapping that
        // produced it. Legacy payload frames predate profiles (the loopback
        // probe) and are exempt — they are already confined to the single
        // translation boundary.
        if !frame.carries_payload() {
            let bound = self
                .clients
                .active_profile(client)
                .is_some_and(|active| active.activation_revision == frame.activation_revision);
            if !bound {
                let generation = self.frame_generation(&frame);
                actions.push(reject_frame(
                    client,
                    &frame,
                    FrameRejectionReason::ProfileMismatch,
                    generation,
                ));
                return;
            }
        }
        let Some(descriptor) =
            crate::capabilities::scope_capability(&self.capabilities, frame.vehicle, &frame.scope)
        else {
            let generation = self.frame_generation(&frame);
            actions.push(reject_frame(
                client,
                &frame,
                FrameRejectionReason::UnknownScope,
                generation,
            ));
            return;
        };
        match crate::command_gate::gate_frame(&frame, descriptor) {
            Ok(typed) => self.accept_frame(typed, sender, client, now, actions),
            Err(reason) => {
                let generation = self.frame_generation(&frame);
                actions.push(reject_frame(client, &frame, reason, generation));
            }
        }
    }

    /// Books an accepted, gate-typed frame: refreshes setpoint freshness,
    /// runs the recovery activation check, and forwards it to the adapter.
    fn accept_frame(
        &mut self,
        frame: ScopedControlFrame,
        sender: PrincipalId,
        client: ClientKey,
        now: MonoTimestamp,
        actions: &mut Actions,
    ) {
        // The holder is actively driving; push its frame-silence deadline
        // forward so the watchdog only fires on real silence. Only an
        // intent-bearing frame counts as setpoint freshness — actions-only
        // traffic proves the client is alive, not that it is commanding the
        // vehicle, and must not hold the lease of a setpoint-silent holder
        // open.
        let pair = self
            .authority_pair(frame.vehicle, &frame.scope)
            .unwrap_or_else(|| (frame.vehicle, frame.scope.clone()));
        // Duplicate and reordered datagrams are refused BEFORE they can
        // refresh liveness or clear recovery: within one generation the
        // sequence must strictly advance (wrap-aware); a fresh generation
        // restarts the domain.
        if !self.sequence_admits(&pair, frame.generation, frame.sequence) {
            let generation = self.frame_generation(&frame);
            actions.push(reject_frame(
                client,
                &frame,
                FrameRejectionReason::StaleSequence,
                generation,
            ));
            return;
        }
        if frame.intent.is_some() {
            self.note_frame_accepted(pair.0, &pair.1, sender, now);
        }
        // A demonstrated-neutral frame from the fenced new holder is the
        // recovery activation condition; the clear (if any) is emitted
        // before the apply so the adapter un-latches first.
        self.maybe_activate_recovery(&frame, actions);
        actions.push(SessionAction::ApplyToAdapter { client, frame });
    }

    /// Admits `sequence` for `pair` at `generation` iff it strictly
    /// advances (wrap-aware) past the last accepted one under the same
    /// generation, recording it; a new generation restarts the domain.
    pub(crate) fn sequence_admits(
        &mut self,
        pair: &crate::clients::ScopePair,
        generation: pilotage_protocol::Generation,
        sequence: pilotage_protocol::SequenceNum,
    ) -> bool {
        let sequence = sequence.as_u32();
        match self.sequences.get_mut(pair) {
            Some((held_generation, last)) if *held_generation == generation => {
                let distance = sequence.wrapping_sub(*last);
                if distance == 0 || distance > u32::MAX / 2 {
                    return false;
                }
                *last = sequence;
                true
            }
            _ => {
                self.sequences.insert(pair.clone(), (generation, sequence));
                true
            }
        }
    }
}

/// Builds a `RejectFrame` action carrying the scope's current generation.
fn reject_frame(
    client: ClientKey,
    frame: &ScopedControlFrame,
    reason: FrameRejectionReason,
    current_generation: Generation,
) -> SessionAction {
    SessionAction::RejectFrame {
        client,
        rejection: FrameRejected {
            vehicle: frame.vehicle,
            scope: frame.scope.clone(),
            sequence: frame.sequence,
            reason,
            current_generation,
        },
    }
}
