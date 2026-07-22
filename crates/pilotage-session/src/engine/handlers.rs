//! Per-message handlers for [`SessionEngine`] (ADR-0005, ADR-0006, ADR-0009).
//!
//! Each handler consumes one decoded message and appends the resulting actions
//! to the shared bounded [`Actions`] accumulator. The handlers hold every
//! handshake, lease, fencing, and staleness decision so the driver stays thin.

use pilotage_authority::{AuthorityCommand, AuthorityEffect, FrameVerdict};
use pilotage_protocol::{
    ClientHello, FrameRejected, FrameRejectionReason, Generation, LeaseDenialReason, LeaseRequest,
    Ping, Pong, PrincipalId, ScopedControlFrame, ServerWelcome,
};
use pilotage_timing::{Freshness, MonoTimestamp};

use crate::action::{CloseReason, LinkLossTrigger, SessionAction};
use crate::capabilities::host_capabilities;
use crate::clients::ScopePair;
use crate::engine::{Actions, SessionEngine};
use crate::message::ClientKey;
use crate::outbound::OutboundMessage;

impl SessionEngine {
    /// Answers a `ClientHello` with a `ServerWelcome`, or closes the
    /// connection on a version mismatch or a duplicate handshake.
    pub(super) fn on_hello(
        &mut self,
        client: ClientKey,
        hello: ClientHello,
        actions: &mut Actions,
    ) {
        let required = self.config.required_protocol_version;
        if hello.protocol_version < required {
            actions.push(SessionAction::CloseClient {
                client,
                reason: CloseReason::UnsupportedProtocolVersion {
                    offered: hello.protocol_version,
                    required,
                },
            });
            return;
        }
        if self.clients.is_welcomed(client) {
            actions.push(SessionAction::CloseClient {
                client,
                reason: CloseReason::DuplicateHello,
            });
            return;
        }
        let state = self.clients.welcome(client);
        let welcome = ServerWelcome {
            session: state.session,
            principal: state.principal,
            host_capabilities: host_capabilities(&self.capabilities, &self.config.host_version),
            scope_holders: self.clients.scope_holders(),
        };
        actions.send(client, OutboundMessage::Welcome(welcome));
    }

    /// Routes a `LeaseRequest` through the authority engine's grant path,
    /// replying with a `LeaseResponse` and broadcasting the grant on success.
    pub(super) fn on_lease(
        &mut self,
        client: ClientKey,
        request: LeaseRequest,
        now: MonoTimestamp,
        actions: &mut Actions,
    ) {
        let Some(principal) = self.welcomed_principal(client, actions) else {
            return;
        };
        let pair: ScopePair = (request.vehicle, request.scope.clone());
        if !self.clients.is_registered(&pair) {
            actions.send(
                client,
                OutboundMessage::LeaseResponse(lease_denied(
                    &request,
                    Generation::new(0),
                    LeaseDenialReason::UnknownScope,
                )),
            );
            return;
        }
        if let Some(current) = self.clients.holder_of(&pair) {
            let generation = self
                .clients
                .generation_of(&pair)
                .unwrap_or_else(|| Generation::new(0));
            // A principal already holding the scope re-requesting it is not an
            // error; reply with the standing grant rather than a denial.
            let response = if current == principal {
                lease_granted(&request, generation)
            } else {
                lease_denied(&request, generation, LeaseDenialReason::AlreadyHeld)
            };
            actions.send(client, OutboundMessage::LeaseResponse(response));
            return;
        }
        let effects = self.authority.handle(
            AuthorityCommand::Grant {
                vehicle: request.vehicle,
                scope: request.scope.clone(),
                to: principal,
            },
            now,
        );
        // The authority engine is the single source of truth for whether the
        // grant actually took effect; a `CommandRejected` effect (the scope
        // was concurrently claimed by another principal between the registry
        // check above and this call) must produce a denial, never a granted
        // response with a stale generation.
        let rejected = effects
            .iter()
            .any(|effect| matches!(effect, AuthorityEffect::CommandRejected { .. }));
        // `fan_out_authority` updates the holder record from the grant effect,
        // so the post-grant generation lookup reflects the advanced value. A
        // grant can also displace a prior holder (revoke effect), which is an
        // authority-driven loss, not silence or a disconnect.
        self.fan_out_authority(effects, now, LinkLossTrigger::AuthorityRevoked, actions);
        let current_generation = self
            .clients
            .generation_of(&pair)
            .unwrap_or_else(|| Generation::new(0));
        let response = if rejected {
            lease_denied(&request, current_generation, LeaseDenialReason::AlreadyHeld)
        } else {
            lease_granted(&request, current_generation)
        };
        actions.send(client, OutboundMessage::LeaseResponse(response));
    }

    /// Routes a voluntary scope release through the authority engine and
    /// acknowledges it (ADR-0006). A successful release advances the
    /// fencing generation and engages the vehicle's link-loss policy —
    /// the same authoritative state an involuntary loss produces — so a
    /// client that latched input loss relinquishes deterministically
    /// instead of waiting out the silence watchdog (which remains the
    /// independent backup). The acknowledgement is unicast on the
    /// reliable bootstrap stream; `released` is false when the sender did
    /// not hold the scope, so a stale or duplicate release is a no-op the
    /// client can observe.
    pub(super) fn on_release(
        &mut self,
        client: ClientKey,
        release: pilotage_protocol::LeaseRelease,
        now: MonoTimestamp,
        actions: &mut Actions,
    ) {
        let Some(principal) = self.welcomed_principal(client, actions) else {
            return;
        };
        let pair: ScopePair = (release.vehicle, release.scope.clone());
        let effects = self.authority.handle(
            AuthorityCommand::Release {
                vehicle: release.vehicle,
                scope: release.scope.clone(),
                by: principal,
            },
            now,
        );
        let released = effects
            .iter()
            .any(|effect| matches!(effect, AuthorityEffect::ScopeLeaseRevoked { .. }));
        self.fan_out_authority(effects, now, LinkLossTrigger::AuthorityRevoked, actions);
        let generation = self
            .clients
            .generation_of(&pair)
            .unwrap_or_else(|| Generation::new(0));
        actions.send(
            client,
            OutboundMessage::LeaseReleased(pilotage_protocol::LeaseReleased {
                vehicle: release.vehicle,
                scope: release.scope,
                released,
                generation,
            }),
        );
    }

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
        // LOOPBACK-ONLY: `sampled_at` and `now` are treated as one monotonic
        // clock (see the crate/engine module docs). A networked host must route
        // this through `pilotage_timing::estimated_age` instead.
        let age = now.saturating_duration_since(frame.sampled_at);
        if let Freshness::Stale { .. } = self.staleness.check(age) {
            let generation = self.frame_generation(&frame);
            actions.push(reject_frame(
                client,
                &frame,
                FrameRejectionReason::TooOld,
                generation,
            ));
            return;
        }
        // Fence on the sender's identity, not just on generation. `verify_frame`
        // confirms a holder exists at the current generation, but generations are
        // broadcast to every client, so a non-holder could otherwise forge an
        // in-generation frame. Only the recorded holder may drive the scope. An
        // unregistered scope has no holder and is left to `verify_frame` so the
        // sender still learns the scope is unknown rather than merely unheld.
        let pair = (frame.vehicle, frame.scope.clone());
        if self.clients.is_registered(&pair) && self.clients.holder_of(&pair) != Some(sender) {
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
            .verify_frame(frame.vehicle, &frame.scope, frame.generation)
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
        if frame.intent.is_some() {
            self.note_frame_accepted(frame.vehicle, &frame.scope, sender, now);
        }
        // A demonstrated-neutral frame from the fenced new holder is the
        // recovery activation condition; the clear (if any) is emitted
        // before the apply so the adapter un-latches first.
        self.maybe_activate_recovery(&frame, actions);
        actions.push(SessionAction::ApplyToAdapter { client, frame });
    }

    /// Records a client's control-profile activation announcement
    /// (INPUT-01): the session-side traceability record binding the
    /// `activation_revision` its frames carry to the profile identity,
    /// document revision, and content digests of both the scheme and the
    /// selected device profile. An announcement before the handshake closes
    /// the connection like any other pre-welcome traffic; one naming a
    /// foreign session, or regressing the monotonic activation revision,
    /// closes it too — a corrupted traceability record is worse than none.
    pub(super) fn on_profile_activation(
        &mut self,
        client: ClientKey,
        activation: pilotage_protocol::ProfileActivation,
        actions: &mut Actions,
    ) {
        if self.welcomed_principal(client, actions).is_none() {
            return;
        }
        let Some(state) = self.clients.get(client) else {
            return;
        };
        if activation.session != state.session {
            actions.push(SessionAction::CloseClient {
                client,
                reason: CloseReason::ProfileSessionMismatch {
                    announced: activation.session,
                    expected: state.session,
                },
            });
            return;
        }
        if let Some(previous) = self.clients.active_profile(client) {
            // Wrapping forward distance: the new revision must advance, and
            // a jump past the half-range reads as a regression, not a leap.
            let advance = activation
                .activation_revision
                .wrapping_sub(previous.activation_revision);
            if advance == 0 || advance > u32::MAX / 2 {
                actions.push(SessionAction::CloseClient {
                    client,
                    reason: CloseReason::NonMonotonicActivation {
                        previous: previous.activation_revision,
                        announced: activation.activation_revision,
                    },
                });
                return;
            }
        }
        self.clients.record_profile_activation(client, activation);
    }

    /// The client's last announced control-profile activation, if any —
    /// the record session telemetry and evidence bind control frames to.
    #[must_use]
    pub fn active_profile(
        &self,
        client: ClientKey,
    ) -> Option<&pilotage_protocol::ProfileActivation> {
        self.clients.active_profile(client)
    }

    /// Answers a `Ping` with a `Pong` echoing the sender sample and stamping
    /// the responder's own time (ADR-0009).
    pub(super) fn on_ping(
        &mut self,
        client: ClientKey,
        ping: Ping,
        now: MonoTimestamp,
        actions: &mut Actions,
    ) {
        if self.welcomed_principal(client, actions).is_none() {
            return;
        }
        let pong = Pong {
            nonce: ping.nonce,
            echoed_sender_sent_at: ping.sender_sent_at,
            responder_sent_at: now,
        };
        actions.send(client, OutboundMessage::Pong(pong));
    }

    /// Releases every scope a disconnecting client held (ADR-0010 link loss).
    pub(super) fn on_disconnect(&mut self, client: ClientKey, actions: &mut Actions) {
        if let Some((principal, scopes)) = self.clients.remove(client) {
            self.release_on_disconnect(principal, scopes, actions);
        }
    }

    /// Returns the requesting client's principal, or emits a close action when
    /// the client has not completed the handshake.
    fn welcomed_principal(
        &self,
        client: ClientKey,
        actions: &mut Actions,
    ) -> Option<pilotage_protocol::PrincipalId> {
        match self.clients.get(client) {
            Some(state) => Some(state.principal),
            None => {
                actions.push(SessionAction::CloseClient {
                    client,
                    reason: CloseReason::HandshakeNotComplete,
                });
                None
            }
        }
    }

    /// Looks up the current generation for a frame's scope, defaulting to zero
    /// for an unknown scope.
    fn frame_generation(&self, frame: &ScopedControlFrame) -> Generation {
        self.clients
            .generation_of(&(frame.vehicle, frame.scope.clone()))
            .unwrap_or_else(|| Generation::new(0))
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

/// Builds a granted `LeaseResponse`.
fn lease_granted(
    request: &LeaseRequest,
    generation: Generation,
) -> pilotage_protocol::LeaseResponse {
    pilotage_protocol::LeaseResponse {
        vehicle: request.vehicle,
        scope: request.scope.clone(),
        granted: true,
        generation,
        reason: None,
    }
}

/// Builds a denied `LeaseResponse` carrying the unchanged current generation.
fn lease_denied(
    request: &LeaseRequest,
    generation: Generation,
    reason: LeaseDenialReason,
) -> pilotage_protocol::LeaseResponse {
    pilotage_protocol::LeaseResponse {
        vehicle: request.vehicle,
        scope: request.scope.clone(),
        granted: false,
        generation,
        reason: Some(reason),
    }
}
