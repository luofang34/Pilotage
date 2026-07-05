//! Per-scope authority state held by the engine (ADR-0006, ADR-0010).
//!
//! One [`ScopeState`] exists per registered `(vehicle, scope)`. It records the
//! fencing generation, the holder disposition, and the effective holder's link
//! state. The engine mutates it only through its own command handlers; nothing
//! here reads a clock.

use pilotage_protocol::{Generation, PrincipalId};

use crate::command::{AuthorityClass, LinkState};

/// Disposition of authority for one scope.
///
/// `Offered` keeps `from` as the effective holder: the offerer retains control
/// until the recipient commits an accept, per ADR-0010's two-phase model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HolderState {
    /// No principal holds the scope.
    Unassigned,
    /// `principal` holds the scope.
    Held {
        /// Effective holder.
        principal: PrincipalId,
        /// Set when the current hold was established by emergency override,
        /// recording the class so a repeat override can be detected as
        /// idempotent.
        override_class: Option<AuthorityClass>,
    },
    /// `from` holds the scope but has offered it to `to`; the offer expires at
    /// `expires_at` in the caller's monotonic domain.
    Offered {
        /// Effective holder for the duration of the offer.
        from: PrincipalId,
        /// Prospective recipient of authority.
        to: PrincipalId,
        /// Monotonic timestamp at which the offer was created.
        offered_at: pilotage_timing::MonoTimestamp,
        /// Monotonic timestamp at which the offer expires.
        expires_at: pilotage_timing::MonoTimestamp,
    },
}

impl HolderState {
    /// Returns the effective holder, if any.
    ///
    /// During `Offered`, the offerer (`from`) is the effective holder.
    pub(crate) fn effective_holder(&self) -> Option<PrincipalId> {
        match self {
            HolderState::Unassigned => None,
            HolderState::Held { principal, .. } => Some(*principal),
            HolderState::Offered { from, .. } => Some(*from),
        }
    }
}

/// Complete authority state for a single registered scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopeState {
    /// Current fencing generation. Never decreases except by `u64` wrap
    /// (ADR-0006); advances via [`Generation::next`].
    pub(crate) generation: Generation,
    /// Current holder disposition.
    pub(crate) holder: HolderState,
    /// Most recently reported link state of the effective holder. Reset to
    /// `Nominal` whenever the scope becomes `Unassigned` or a new holder is
    /// installed.
    pub(crate) link: LinkState,
}

impl ScopeState {
    /// Creates freshly registered state: `Unassigned`, generation zero.
    pub(crate) fn new() -> Self {
        Self {
            generation: Generation::new(0),
            holder: HolderState::Unassigned,
            link: LinkState::Nominal,
        }
    }

    /// Returns the effective holder, if any.
    pub(crate) fn effective_holder(&self) -> Option<PrincipalId> {
        self.holder.effective_holder()
    }

    /// Advances the fencing generation via wrapping successor.
    pub(crate) fn advance_generation(&mut self) {
        self.generation = self.generation.next();
    }

    /// Reverts a pending offer to `Held(from)` without advancing the
    /// generation (no transfer occurred), returning the holder the scope
    /// reverted to. Returns `None` when there is no pending offer.
    ///
    /// TTL expiry and a late accept arriving at or after the deadline share
    /// this transition so both paths return the scope to the offerer
    /// identically (ADR-0010: expiry returns the scope to `Held(A)`).
    pub(crate) fn expire_offer(&mut self) -> Option<PrincipalId> {
        let HolderState::Offered { from, .. } = &self.holder else {
            return None;
        };
        let holder = *from;
        self.holder = HolderState::Held {
            principal: holder,
            override_class: None,
        };
        self.link = LinkState::Nominal;
        Some(holder)
    }
}
