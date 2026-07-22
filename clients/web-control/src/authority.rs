//! Fenced authority state and lease planning for every client control scope.

use crate::plan::LeaseAction;

/// The three independent authority domains the browser control runtime owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityScope {
    /// The motion authority group shared by velocity and direct-flight members.
    Motion,
    /// The independently fenced gimbal pointing scope.
    Gimbal,
    /// The on-demand simulator lifecycle scope.
    Lifecycle,
}

impl AuthorityScope {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 3] = [Self::Motion, Self::Gimbal, Self::Lifecycle];

    /// Decodes the stable wasm ABI scope code.
    #[must_use]
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::Motion),
            1 => Some(Self::Gimbal),
            2 => Some(Self::Lifecycle),
            _ => None,
        }
    }

    /// Returns the stable wasm ABI scope code.
    #[must_use]
    pub const fn code(self) -> u32 {
        self as u32
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// One reliable-stream authority transition after vehicle/scope filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityEvent {
    /// The host granted the scope on this generation.
    LeaseGranted {
        /// The granted generation.
        generation: u64,
    },
    /// The host denied the lease request; denial is terminal for the session.
    LeaseDenied,
    /// The host acknowledged a release and supplied its fencing generation.
    LeaseReleased {
        /// The host's current fencing generation.
        generation: u64,
    },
    /// The host rejected a frame as stale or unleased and supplied its fence.
    Revoked {
        /// The host's current fencing generation.
        generation: u64,
    },
    /// The host confirmed link-loss recovery on this generation.
    LinkLossCleared {
        /// The recovered motion generation.
        generation: u64,
    },
    /// The adapter admitted the frame but could not enact it without arming.
    UplinkIdle,
    /// A reliable control action completed.
    ActionResult {
        /// The protocol action code.
        action: u32,
        /// Whether the host accepted the action.
        accepted: bool,
    },
}

/// How an authority event affected its scope slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityDisposition {
    /// The event was unrelated, duplicate, or blocked by terminal denial.
    Ignored,
    /// The event changed authoritative state.
    Applied,
    /// A grant did not clear the active generation fence.
    Stale,
}

impl AuthorityDisposition {
    /// Returns the stable wasm ABI disposition code.
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            Self::Ignored => 0,
            Self::Applied => 1,
            Self::Stale => 2,
        }
    }
}

/// The authoritative state of one scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityState {
    generation: u64,
    fence: u64,
    granted: bool,
    denied: bool,
    recovered: bool,
    needs_arm: bool,
}

impl AuthorityState {
    /// The generation currently authorized for frames and actions.
    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }

    /// The last host-confirmed fencing generation.
    #[must_use]
    pub const fn fence(self) -> u64 {
        self.fence
    }

    /// Whether the scope is currently granted.
    #[must_use]
    pub const fn granted(self) -> bool {
        self.granted
    }

    /// Whether lease denial is terminal for the current session.
    #[must_use]
    pub const fn denied(self) -> bool {
        self.denied
    }

    /// Whether the host has confirmed recovery on the current generation.
    #[must_use]
    pub const fn recovered(self) -> bool {
        self.recovered
    }

    /// Whether granted motion still needs an accepted arm action to enact.
    #[must_use]
    pub const fn needs_arm(self) -> bool {
        self.needs_arm
    }

    /// Packs the wasm ABI flags: granted, denied, recovered, and needs-arm.
    #[must_use]
    pub fn flags(self) -> u32 {
        u32::from(self.granted)
            | (u32::from(self.denied) << 1)
            | (u32::from(self.recovered) << 2)
            | (u32::from(self.needs_arm) << 3)
    }
}

impl Default for AuthorityState {
    fn default() -> Self {
        Self {
            generation: 0,
            fence: 0,
            granted: false,
            denied: false,
            recovered: true,
            needs_arm: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingAction {
    Request,
    Release,
}

#[derive(Debug, Clone, Copy)]
struct ScopeSlot {
    state: AuthorityState,
    fence_active: bool,
    pending: Option<PendingAction>,
    last_action_ms: f64,
}

impl Default for ScopeSlot {
    fn default() -> Self {
        Self {
            state: AuthorityState::default(),
            fence_active: false,
            pending: None,
            last_action_ms: f64::NEG_INFINITY,
        }
    }
}

/// The runtime-owned table that applies events and emits debounced lease plans.
#[derive(Debug, Default)]
pub(crate) struct AuthorityTable {
    slots: [ScopeSlot; 3],
}

impl AuthorityTable {
    pub(crate) fn begin_session(&mut self) {
        self.slots = [ScopeSlot::default(); 3];
    }

    pub(crate) fn state(&self, scope: AuthorityScope) -> AuthorityState {
        self.slots[scope.index()].state
    }

    pub(crate) fn apply(
        &mut self,
        scope: AuthorityScope,
        event: AuthorityEvent,
    ) -> AuthorityDisposition {
        let slot = &mut self.slots[scope.index()];
        match event {
            AuthorityEvent::LeaseGranted { generation } => apply_grant(slot, generation),
            AuthorityEvent::LeaseDenied => {
                slot.pending = None;
                slot.state.granted = false;
                slot.state.denied = true;
                AuthorityDisposition::Applied
            }
            AuthorityEvent::LeaseReleased { generation }
            | AuthorityEvent::Revoked { generation } => {
                if fence_event_is_stale(slot, generation) {
                    return AuthorityDisposition::Ignored;
                }
                slot.pending = None;
                slot.state.granted = false;
                slot.state.fence = generation;
                slot.fence_active = true;
                if scope == AuthorityScope::Motion {
                    slot.state.recovered = false;
                }
                AuthorityDisposition::Applied
            }
            AuthorityEvent::LinkLossCleared { generation } => {
                if scope != AuthorityScope::Motion
                    || !slot.state.granted
                    || generation != slot.state.generation
                {
                    return AuthorityDisposition::Ignored;
                }
                slot.state.recovered = true;
                AuthorityDisposition::Applied
            }
            AuthorityEvent::UplinkIdle => {
                if scope != AuthorityScope::Motion {
                    return AuthorityDisposition::Ignored;
                }
                slot.state.needs_arm = true;
                AuthorityDisposition::Applied
            }
            AuthorityEvent::ActionResult { action, accepted } => {
                apply_action_result(scope, slot, action, accepted)
            }
        }
    }

    pub(crate) fn plan(
        &mut self,
        scope: AuthorityScope,
        desired: bool,
        now_ms: f64,
    ) -> Option<LeaseAction> {
        let slot = &mut self.slots[scope.index()];
        if desired {
            plan_request(scope, slot, now_ms)
        } else {
            plan_release(slot, now_ms)
        }
    }

    pub(crate) fn plan_explicit(
        &mut self,
        scope: AuthorityScope,
        desired: bool,
        now_ms: f64,
    ) -> Option<LeaseAction> {
        let slot = &mut self.slots[scope.index()];
        if desired && !slot.state.granted && !slot.state.denied {
            slot.pending = None;
        }
        self.plan(scope, desired, now_ms)
    }
}

fn fence_event_is_stale(slot: &ScopeSlot, generation: u64) -> bool {
    if slot.state.granted
        && generation != slot.state.generation
        && is_fresh_generation(slot.state.generation, generation)
    {
        return true;
    }
    slot.fence_active
        && generation != slot.state.fence
        && is_fresh_generation(slot.state.fence, generation)
}

fn apply_grant(slot: &mut ScopeSlot, generation: u64) -> AuthorityDisposition {
    if slot.state.denied {
        return AuthorityDisposition::Ignored;
    }
    if slot.state.granted && slot.state.generation == generation {
        return AuthorityDisposition::Ignored;
    }
    if slot.fence_active && !is_fresh_generation(generation, slot.state.fence) {
        return AuthorityDisposition::Stale;
    }
    slot.pending = None;
    slot.state.generation = generation;
    slot.state.granted = true;
    AuthorityDisposition::Applied
}

/// The wire `ControlAction` codes the needs-arm latch reacts to (the shell
/// forwards them from `CONTROL_ACTION` unchanged): an accepted arm restarts
/// the uplink stream, an accepted disarm stops it.
const ACTION_ARM: u32 = 1;
const ACTION_DISARM: u32 = 2;

fn apply_action_result(
    scope: AuthorityScope,
    slot: &mut ScopeSlot,
    action: u32,
    accepted: bool,
) -> AuthorityDisposition {
    if scope != AuthorityScope::Motion || !accepted {
        return AuthorityDisposition::Ignored;
    }
    match action {
        ACTION_ARM => slot.state.needs_arm = false,
        ACTION_DISARM => slot.state.needs_arm = true,
        _ => return AuthorityDisposition::Ignored,
    }
    AuthorityDisposition::Applied
}

fn plan_request(scope: AuthorityScope, slot: &mut ScopeSlot, now_ms: f64) -> Option<LeaseAction> {
    if slot.state.granted || slot.state.denied || slot.pending == Some(PendingAction::Release) {
        return None;
    }
    let retry_ms = if scope == AuthorityScope::Motion {
        250.0
    } else {
        3000.0
    };
    if slot.pending == Some(PendingAction::Request) && now_ms - slot.last_action_ms < retry_ms {
        return None;
    }
    slot.pending = Some(PendingAction::Request);
    slot.last_action_ms = now_ms;
    Some(LeaseAction::Request)
}

fn plan_release(slot: &mut ScopeSlot, now_ms: f64) -> Option<LeaseAction> {
    if !slot.state.granted {
        if slot.pending == Some(PendingAction::Request) {
            slot.pending = None;
        }
        return None;
    }
    if slot.pending == Some(PendingAction::Release) && now_ms - slot.last_action_ms < 250.0 {
        return None;
    }
    slot.pending = Some(PendingAction::Release);
    slot.last_action_ms = now_ms;
    Some(LeaseAction::Release)
}

/// Whether `candidate` is strictly newer than `fence` in modular u64 order.
#[must_use]
pub const fn is_fresh_generation(candidate: u64, fence: u64) -> bool {
    let delta = candidate.wrapping_sub(fence);
    delta != 0 && delta < (1_u64 << 63)
}

#[cfg(test)]
mod tests;
