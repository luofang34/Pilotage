//! Effects emitted by the authority engine and the per-frame verdict type.
//!
//! Effects are the engine's only output. They map onto the ADR-0012 authority
//! event vocabulary (`ScopeLeaseGranted`, `ScopeTransferOffered`,
//! `ScopeTransferCommitted`, `ScopeLeaseRevoked`, `EmergencyOverrideApplied`,
//! and the warning/link signals) plus a typed [`CommandRejected`] used when a
//! command cannot be honored.
//!
//! [`CommandRejected`]: AuthorityEffect::CommandRejected

use pilotage_protocol::{Generation, PrincipalId, ScopeId, VehicleId};

use crate::command::{AuthorityClass, LinkState, OverrideReason};

/// An effect emitted in response to a command or a timer expiry.
///
/// Effects are ordered: for a single command the engine emits them in the
/// order the embedding host should observe and persist them. Authority effects
/// are always persisted per ADR-0012; they are the audit trail for handover
/// and override disputes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityEffect {
    /// A `(vehicle, scope)` became known and is now `Unassigned`.
    ScopeRegistered {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Newly registered scope.
        scope: ScopeId,
    },
    /// A lease was granted, making `holder` the effective holder at
    /// `generation` (ADR-0012 `ScopeLeaseGranted`).
    ScopeLeaseGranted {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope granted.
        scope: ScopeId,
        /// New effective holder.
        holder: PrincipalId,
        /// Generation in effect after the grant.
        generation: Generation,
    },
    /// A transfer was offered from `from` to `to` (ADR-0012
    /// `ScopeTransferOffered`); `from` remains the effective holder until the
    /// offer is accepted or expires.
    ScopeTransferOffered {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope offered.
        scope: ScopeId,
        /// Current effective holder making the offer.
        from: PrincipalId,
        /// Prospective recipient.
        to: PrincipalId,
        /// Generation in effect while the offer is pending (unchanged).
        generation: Generation,
        /// Timestamp, in the caller's monotonic domain, at which the offer
        /// expires if not accepted.
        expires_at: pilotage_timing::MonoTimestamp,
    },
    /// A transfer was committed: `to` is now the effective holder at
    /// `generation` (ADR-0012 `ScopeTransferCommitted`). This is the atomic
    /// commit point of a normal handover.
    ScopeTransferCommitted {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope transferred.
        scope: ScopeId,
        /// Previous effective holder.
        from: PrincipalId,
        /// New effective holder.
        to: PrincipalId,
        /// Generation in effect after the commit.
        generation: Generation,
    },
    /// A pending offer expired and the scope returned to its offerer
    /// (ADR-0010 TTL). The generation is unchanged: no transfer occurred.
    ScopeTransferExpired {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope whose offer expired.
        scope: ScopeId,
        /// Holder the scope reverted to.
        holder: PrincipalId,
        /// Generation in effect after reverting (unchanged).
        generation: Generation,
    },
    /// A lease was revoked or released and the scope is now `Unassigned`
    /// (ADR-0012 `ScopeLeaseRevoked`).
    ScopeLeaseRevoked {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope revoked.
        scope: ScopeId,
        /// Previous effective holder, if any.
        previous_holder: Option<PrincipalId>,
        /// Generation in effect after revocation.
        generation: Generation,
    },
    /// An emergency override took effect, displacing any previous holder
    /// (ADR-0012 `EmergencyOverrideApplied`).
    EmergencyOverrideApplied {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope seized.
        scope: ScopeId,
        /// Previous effective holder, if any.
        previous_holder: Option<PrincipalId>,
        /// Principal that seized authority.
        holder: PrincipalId,
        /// Authority class justifying the override.
        authority_class: AuthorityClass,
        /// Recorded reason.
        reason: OverrideReason,
        /// Generation in effect after the override.
        generation: Generation,
    },
    /// A repeated emergency override by the current override holder with the
    /// same class had no additional effect (idempotency, ADR-0010). The
    /// generation is unchanged.
    EmergencyOverrideAlreadyEffective {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope already held under override.
        scope: ScopeId,
        /// Principal already holding the override.
        holder: PrincipalId,
        /// Authority class of the standing override.
        authority_class: AuthorityClass,
        /// Generation in effect (unchanged).
        generation: Generation,
    },
    /// A holder's link state changed (ADR-0012 `LinkStateChanged`).
    LinkStateChanged {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope whose holder's link changed.
        scope: ScopeId,
        /// Principal whose link changed.
        principal: PrincipalId,
        /// New link state.
        state: LinkState,
    },
    /// The effective holder's link was lost, triggering the v1 link-loss
    /// release policy (ADR-0010, ADR-0012 `LinkStateChanged` +
    /// `WarningRaised`). The scope is released and the generation advances.
    HolderLinkLost {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope whose holder was lost.
        scope: ScopeId,
        /// Principal that was lost.
        lost_holder: PrincipalId,
        /// Generation in effect after the release.
        generation: Generation,
    },
    /// A non-fatal warning about a late, duplicate, or contradictory command
    /// that produced no state change (ADR-0012 `WarningRaised`). Confirmations
    /// arriving out of order are the canonical source.
    WarningRaised {
        /// Vehicle owning the scope.
        vehicle: VehicleId,
        /// Scope the warning concerns.
        scope: ScopeId,
        /// Machine-readable warning classification.
        warning: AuthorityWarning,
    },
    /// A command could not be honored (typed reason). No state changed.
    CommandRejected {
        /// Vehicle owning the scope, if the command carried one.
        vehicle: VehicleId,
        /// Scope the command targeted.
        scope: ScopeId,
        /// Typed rejection reason.
        reason: RejectReason,
    },
}

/// Classification of a warning-level, non-mutating authority event
/// (ADR-0012 `WarningRaised`).
///
/// Warnings are emitted when a command is well-formed and targets a known
/// scope but arrives too late or contradicts committed state, so it changes
/// nothing. They are audited and surfaced in the UI, never rolled back.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AuthorityWarning {
    /// An "I have control" confirmation arrived from a principal that is not
    /// the current effective holder (late, duplicate, or contradictory).
    UnexpectedIHave {
        /// Principal that issued the confirmation.
        by: PrincipalId,
        /// Current effective holder, if any.
        current_holder: Option<PrincipalId>,
    },
    /// A "you have control" confirmation arrived from a principal that is not
    /// the current effective holder (late, duplicate, or contradictory).
    UnexpectedYouHave {
        /// Principal that issued the confirmation.
        by: PrincipalId,
        /// Current effective holder, if any.
        current_holder: Option<PrincipalId>,
    },
}

/// Typed reason a command was rejected without changing state.
///
/// Each variant carries the context its human-facing message needs, per the
/// workspace error-handling policy: rejections are never a bare boolean.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RejectReason {
    /// The targeted `(vehicle, scope)` has not been registered.
    UnknownScope,
    /// The scope was already registered when a register was requested.
    ScopeAlreadyRegistered,
    /// A grant targeted a scope that is not `Unassigned`.
    ScopeNotUnassigned {
        /// Current effective holder blocking the grant.
        current_holder: PrincipalId,
    },
    /// An offer, release, or similar command came from a principal that is not
    /// the effective holder.
    NotCurrentHolder {
        /// Principal that issued the command.
        actor: PrincipalId,
        /// Current effective holder, if any.
        current_holder: Option<PrincipalId>,
    },
    /// A command that requires a held scope found it `Unassigned`.
    ScopeUnassigned,
    /// An offer was requested but the scope already has a pending offer.
    OfferAlreadyPending,
    /// An accept referenced a scope that has no pending offer (never offered,
    /// already accepted, or already expired).
    NoPendingOffer,
    /// An accept came from a principal other than the offer recipient.
    NotOfferRecipient {
        /// Principal that attempted to accept.
        actor: PrincipalId,
        /// Principal the offer was addressed to.
        expected: PrincipalId,
    },
    /// An accept carried a generation that does not match the scope's current
    /// generation (stale accept — fenced out).
    GenerationMismatch {
        /// Generation the acceptor supplied.
        supplied: Generation,
        /// Generation currently in effect.
        current: Generation,
    },
}

/// The result of verifying a single control frame against current authority
/// (ADR-0006 per-frame fencing).
///
/// The verifier operates only on compact session-local state and performs no
/// external policy lookups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameVerdict {
    /// The frame targets the current holder at the current generation and is
    /// accepted. During a pending offer, frames from the offerer at the
    /// current generation are accepted.
    Accepted,
    /// The frame's generation does not equal the scope's current generation.
    RejectedStaleGeneration {
        /// Generation currently in effect for the scope.
        current: Generation,
    },
    /// The scope is known but currently has no holder.
    RejectedNoHolder,
    /// The targeted `(vehicle, scope)` is not registered.
    RejectedUnknownScope,
}
