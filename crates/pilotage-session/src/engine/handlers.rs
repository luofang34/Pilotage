//! Per-message handlers for [`SessionEngine`] (ADR-0005, ADR-0006, ADR-0009).
//!
//! Each handler consumes one decoded message and appends the resulting actions
//! to the shared bounded [`Actions`] accumulator. The handlers hold every
//! handshake, lease, fencing, and staleness decision so the driver stays thin.

use pilotage_authority::{AuthorityCommand, AuthorityEffect};
use pilotage_protocol::{
    ClientHello, Generation, LeaseDenialReason, LeaseRequest, Ping, Pong, ScopedControlFrame,
    ServerWelcome,
};
use pilotage_timing::MonoTimestamp;

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
        // Leasing operates on the scope's exclusive-authority GROUP: sibling
        // scopes drive the same actuator and can never be held apart.
        let Some(pair) = self.authority_pair(request.vehicle, &request.scope) else {
            actions.send(
                client,
                OutboundMessage::LeaseResponse(lease_denied(
                    &request,
                    Generation::new(0),
                    LeaseDenialReason::UnknownScope,
                )),
            );
            return;
        };
        if let Some(current) = self.clients.holder_of(&pair) {
            let response = self.standing_holder_response(&pair, current, principal, &request);
            actions.send(client, OutboundMessage::LeaseResponse(response));
            return;
        }
        let effects = self.authority.handle(
            AuthorityCommand::Grant {
                vehicle: pair.0,
                scope: pair.1.clone(),
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
            // Bind the group lease to the CONCRETE member scope it was
            // acquired for: the holder commands that member and only that
            // member. (The grant's effects cleared any previous binding.)
            self.clients
                .set_held_member(pair.clone(), request.scope.clone());
            lease_granted(&request, current_generation)
        };
        actions.send(client, OutboundMessage::LeaseResponse(response));
    }

    /// The reply to a lease request on an already-held group: a principal
    /// re-requesting the MEMBER it leased gets the standing grant.
    /// Requesting a SIBLING while holding is a scope switch and must go
    /// release-first — neutralize, re-fence, fresh generation — so it is
    /// denied like any other conflicting claim on the group.
    fn standing_holder_response(
        &self,
        pair: &ScopePair,
        current: pilotage_protocol::PrincipalId,
        principal: pilotage_protocol::PrincipalId,
        request: &LeaseRequest,
    ) -> pilotage_protocol::LeaseResponse {
        let generation = self
            .clients
            .generation_of(pair)
            .unwrap_or_else(|| Generation::new(0));
        let same_member = self
            .clients
            .held_member(pair)
            .is_some_and(|member| *member == request.scope);
        if current == principal && same_member {
            lease_granted(request, generation)
        } else {
            lease_denied(request, generation, LeaseDenialReason::AlreadyHeld)
        }
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
        // Releases resolve through the same exclusive-authority group the
        // grant used; an undeclared scope releases nothing (`released:
        // false` below tells the sender so).
        let pair = self
            .authority_pair(release.vehicle, &release.scope)
            .unwrap_or_else(|| (release.vehicle, release.scope.clone()));
        let effects = self.authority.handle(
            AuthorityCommand::Release {
                vehicle: pair.0,
                scope: pair.1.clone(),
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
        self.clients
            .record_profile_activation(client, activation.clone());
        actions.push(SessionAction::ActivationAccepted { client, activation });
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
    pub(super) fn welcomed_principal(
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
    pub(super) fn frame_generation(&self, frame: &ScopedControlFrame) -> Generation {
        let pair = self
            .authority_pair(frame.vehicle, &frame.scope)
            .unwrap_or_else(|| (frame.vehicle, frame.scope.clone()));
        self.clients
            .generation_of(&pair)
            .unwrap_or_else(|| Generation::new(0))
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
