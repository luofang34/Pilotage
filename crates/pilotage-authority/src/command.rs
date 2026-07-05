//! Command vocabulary submitted to the authority engine (ADR-0006, ADR-0010).
//!
//! Commands are the sole input to [`AuthorityEngine::handle`]; every effective
//! authority change originates from one of these variants. The engine owns no
//! I/O and reads no clock, so each command that can advance time-dependent
//! state is handled with an explicit `now` supplied by the caller.
//!
//! [`AuthorityEngine::handle`]: crate::AuthorityEngine::handle

use core::time::Duration;

use pilotage_protocol::{Generation, PrincipalId, ScopeId, VehicleId};

/// The authority class an actor exercises for privileged operations
/// (ADR-0006, ADR-0010).
///
/// Authority class gates which principals may revoke or emergency-override a
/// scope. The precise policy matrix (which class may override which scope) is
/// an open product question tracked in ADR-0010; the engine records and
/// propagates the class but does not itself enforce a class hierarchy in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AuthorityClass {
    /// A peer operator exercising ordinary control authority.
    Operator,
    /// A supervisor or instructor exercising elevated authority.
    Supervisor,
    /// An administrator exercising the highest routine authority.
    Administrator,
    /// An automation agent acting on a policy's behalf.
    Automation,
}

/// Reported liveness of a holder's control link (ADR-0010 link loss).
///
/// The engine tracks the most recent `LinkState` for the effective holder of a
/// scope. Transition to [`LinkState::Lost`] for the effective holder triggers
/// the v1 link-loss policy: release the scope and advance the generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LinkState {
    /// The link is healthy.
    Nominal,
    /// The link is impaired but still carrying control.
    Degraded,
    /// The link is lost; the holder can no longer be assumed to be in control.
    Lost,
}

/// A command submitted to the authority engine for a single
/// `(vehicle, scope)` pair.
///
/// Each command carries the `(vehicle, scope)` it targets so a single engine
/// can multiplex many scopes. Handling a command yields a sequence of
/// [`AuthorityEffect`]s and never blocks or performs I/O.
///
/// [`AuthorityEffect`]: crate::AuthorityEffect
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityCommand {
    /// Register a `(vehicle, scope)` so it becomes known and `Unassigned`.
    ///
    /// Frames and commands targeting an unregistered scope are rejected as
    /// unknown until this command has been applied.
    RegisterScope {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope to register.
        scope: ScopeId,
    },
    /// Grant an unassigned scope directly to a principal, making it `Held`.
    Grant {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope to grant.
        scope: ScopeId,
        /// Principal that becomes the effective holder.
        to: PrincipalId,
    },
    /// Offer a held scope to another principal (normal handover, phase one).
    ///
    /// The offering principal remains the effective holder until the recipient
    /// commits with [`AuthorityCommand::Accept`]; the offer expires after
    /// `ttl` measured from the `now` at which the offer was handled.
    Offer {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope being offered.
        scope: ScopeId,
        /// Current effective holder making the offer.
        from: PrincipalId,
        /// Prospective recipient of authority.
        to: PrincipalId,
        /// Lifetime of the offer before it expires back to the offerer.
        ttl: Duration,
    },
    /// Accept a pending offer (normal handover, phase two — the atomic commit).
    ///
    /// This is the single point at which effective authority changes for a
    /// normal handover (ADR-0010). The accept is honored only if `by` matches
    /// the offer recipient and `expected_generation` matches the scope's
    /// current generation; on commit the scope becomes `Held(by)` and the
    /// generation advances.
    Accept {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope being accepted.
        scope: ScopeId,
        /// Principal accepting authority (must be the offer recipient).
        by: PrincipalId,
        /// Generation the acceptor believes is current, for fencing.
        expected_generation: Generation,
    },
    /// Confirm "I have control" (the recipient's post-commit callout).
    ///
    /// Purely an audit/UI confirmation; it never gates or reverses a transfer.
    ConfirmIHave {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope being confirmed.
        scope: ScopeId,
        /// Principal issuing the confirmation.
        by: PrincipalId,
    },
    /// Confirm "you have control" (the previous holder's post-commit callout).
    ///
    /// Purely an audit/UI confirmation; it never gates or reverses a transfer.
    ConfirmYouHave {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope being confirmed.
        scope: ScopeId,
        /// Principal issuing the confirmation.
        by: PrincipalId,
    },
    /// Voluntarily release a held scope, returning it to `Unassigned`.
    Release {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope being released.
        scope: ScopeId,
        /// Principal releasing (must be the effective holder).
        by: PrincipalId,
    },
    /// Administratively revoke a scope, returning it to `Unassigned`.
    ///
    /// Unlike [`AuthorityCommand::Release`], revocation does not require the
    /// actor to be the current holder; it is a privileged operation carrying
    /// an [`AuthorityClass`].
    Revoke {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope being revoked.
        scope: ScopeId,
        /// Authority class the revoker exercises.
        authority_class: AuthorityClass,
    },
    /// Forcibly seize a scope regardless of current holder (ADR-0010).
    ///
    /// Emergency override is idempotent: a repeat by the current override
    /// holder with the same class re-affirms without advancing the generation.
    EmergencyOverride {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope being seized.
        scope: ScopeId,
        /// Principal seizing authority.
        by: PrincipalId,
        /// Authority class justifying the override.
        authority_class: AuthorityClass,
        /// Human-facing reason recorded in the audit trail.
        reason: OverrideReason,
    },
    /// Report a change in a principal's control-link liveness (ADR-0010).
    HolderLinkChanged {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope whose holder's link changed.
        scope: ScopeId,
        /// Principal whose link changed.
        principal: PrincipalId,
        /// New link state.
        state: LinkState,
    },
}

/// A recorded, human-facing reason for an emergency override (ADR-0010).
///
/// Stored as an owned string so the audit trail preserves operator-entered
/// context; the engine never interprets its contents.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OverrideReason(String);

impl OverrideReason {
    /// Constructs an override reason from any string-like value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the reason as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
