//! Per-message handlers for [`SessionEngine`] (ADR-0005, ADR-0006, ADR-0009).
//!
//! Each handler consumes one decoded message and appends the resulting actions
//! to the shared bounded [`Actions`] accumulator. The handlers hold every
//! handshake, lease, fencing, and staleness decision so the driver stays thin.

use pilotage_authority::{AuthorityCommand, AuthorityEffect, FrameVerdict};
use pilotage_protocol::{
    ClientHello, FrameRejected, FrameRejectionReason, Generation, LeaseDenialReason, LeaseRequest,
    Ping, Pong, ScopedControlFrame, ServerWelcome,
};
use pilotage_timing::{Freshness, MonoTimestamp};

use crate::action::{CloseReason, SessionAction};
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
        // so the post-grant generation lookup reflects the advanced value.
        self.fan_out_authority(effects, actions);
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
            FrameVerdict::Accepted => {
                actions.push(SessionAction::ApplyToAdapter { frame });
            }
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
