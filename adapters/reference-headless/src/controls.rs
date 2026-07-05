//! Latest-valid-value control storage and link-loss hold/neutralize timing.

use pilotage_adapter_api::LinkLossPolicy;
use serde::{Deserialize, Serialize};

/// Logical axis id for throttle, on scope `vehicle.motion`.
pub const THROTTLE_AXIS: u16 = 2;
/// Logical axis id for yaw/steering, on scope `vehicle.motion`.
pub const STEERING_AXIS: u16 = 3;
/// The only control scope the skiff exposes.
pub const MOTION_SCOPE: &str = "vehicle.motion";

/// Latest-valid-value control state plus link-loss bookkeeping for one
/// vehicle.
///
/// `step` consumes `throttle`/`steering` every tick; `apply_control` only
/// overwrites them, never accumulates, matching ADR-0009's latest-value
/// semantics for continuous axes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ControlState {
    /// Latest throttle command in `[-1.0, 1.0]`.
    pub throttle: f64,
    /// Latest steering command in `[-1.0, 1.0]`.
    pub steering: f64,
    /// Current link-loss policy for this vehicle; `None` means the link is
    /// considered live and controls apply normally.
    pub policy: Option<LinkLossPolicy>,
    /// Ticks remaining before a `HoldBrief` policy neutralizes controls;
    /// `None` when no hold is in progress.
    pub hold_ticks_remaining: Option<u32>,
}

impl Default for ControlState {
    fn default() -> Self {
        Self {
            throttle: 0.0,
            steering: 0.0,
            policy: None,
            hold_ticks_remaining: None,
        }
    }
}

impl ControlState {
    /// Overwrites the stored throttle and steering with the latest values.
    pub fn apply(&mut self, throttle: f64, steering: f64) {
        self.throttle = throttle;
        self.steering = steering;
    }

    /// Sets or clears the link-loss policy, arming a `HoldBrief` countdown
    /// if applicable.
    ///
    /// `None` signals link recovery: it clears any engaged policy and any
    /// in-progress hold countdown, so a fresh `apply` takes effect on the
    /// very next `tick_link_loss` instead of being neutralized forever.
    pub fn set_policy(&mut self, policy: Option<LinkLossPolicy>) {
        self.hold_ticks_remaining = match policy {
            Some(LinkLossPolicy::HoldBrief { ticks }) => Some(ticks),
            _ => None,
        };
        self.policy = policy;
    }

    /// Applies one tick of link-loss policy bookkeeping, returning the
    /// controls to actually drive the dynamics with this tick.
    ///
    /// Called every tick: with no policy engaged, controls pass through
    /// unmodified. `Neutralize` zeroes immediately, `HoldBrief` counts down
    /// then neutralizes, and the remaining policies leave controls
    /// untouched here (braking and pause are dynamics-level concerns for a
    /// richer vehicle model, and automation hand-off is a host-level
    /// concern outside this adapter).
    pub fn tick_link_loss(&mut self) -> (f64, f64) {
        match self.policy {
            None => {}
            Some(LinkLossPolicy::Neutralize) => {
                self.throttle = 0.0;
                self.steering = 0.0;
            }
            Some(LinkLossPolicy::HoldBrief { .. }) => {
                // Once the countdown reaches zero the policy is expired, not
                // cleared: `hold_ticks_remaining` stays `None` from then on,
                // but `policy` is still engaged, so every subsequent tick
                // keeps neutralizing rather than letting a later
                // `apply_control` sneak fresh values through. Only an
                // explicit `set_policy(None)` (link recovery) resets this.
                match self.hold_ticks_remaining {
                    Some(0) => {
                        self.throttle = 0.0;
                        self.steering = 0.0;
                        self.hold_ticks_remaining = None;
                    }
                    Some(remaining) => {
                        self.hold_ticks_remaining = Some(remaining - 1);
                    }
                    None => {
                        self.throttle = 0.0;
                        self.steering = 0.0;
                    }
                }
            }
            Some(
                LinkLossPolicy::Brake | LinkLossPolicy::Pause | LinkLossPolicy::EngageAutomation,
            ) => {}
        }
        (self.throttle, self.steering)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::ControlState;
    use pilotage_adapter_api::LinkLossPolicy;

    #[test]
    fn apply_overwrites_latest_value() {
        let mut state = ControlState::default();
        state.apply(0.5, -0.5);
        state.apply(1.0, 0.0);
        assert_eq!(state.throttle, 1.0);
        assert_eq!(state.steering, 0.0);
    }

    #[test]
    fn neutralize_zeroes_controls_every_tick() {
        let mut state = ControlState::default();
        state.apply(1.0, 1.0);
        state.set_policy(Some(LinkLossPolicy::Neutralize));
        let (throttle, steering) = state.tick_link_loss();
        assert_eq!(throttle, 0.0);
        assert_eq!(steering, 0.0);
    }

    #[test]
    fn hold_brief_holds_then_neutralizes() {
        let mut state = ControlState::default();
        state.apply(1.0, 0.5);
        state.set_policy(Some(LinkLossPolicy::HoldBrief { ticks: 2 }));

        let (t0, s0) = state.tick_link_loss();
        assert_eq!((t0, s0), (1.0, 0.5));
        let (t1, s1) = state.tick_link_loss();
        assert_eq!((t1, s1), (1.0, 0.5));
        let (t2, s2) = state.tick_link_loss();
        assert_eq!((t2, s2), (0.0, 0.0));
        let (t3, s3) = state.tick_link_loss();
        assert_eq!((t3, s3), (0.0, 0.0));
    }

    #[test]
    fn clearing_policy_restores_normal_control() {
        let mut state = ControlState::default();
        state.apply(1.0, 1.0);
        state.set_policy(Some(LinkLossPolicy::Neutralize));
        let (neutralized_throttle, neutralized_steering) = state.tick_link_loss();
        assert_eq!((neutralized_throttle, neutralized_steering), (0.0, 0.0));

        // Link recovery: clearing the policy must let a fresh `apply` take
        // effect on the very next tick, not stay neutralized forever.
        state.set_policy(None);
        state.apply(0.8, -0.2);
        let (throttle, steering) = state.tick_link_loss();
        assert_eq!((throttle, steering), (0.8, -0.2));
        assert_eq!(state.policy, None);
        assert_eq!(state.hold_ticks_remaining, None);
    }

    #[test]
    fn hold_brief_expiry_stays_neutralized_across_new_apply() {
        let mut state = ControlState::default();
        state.apply(1.0, 0.5);
        state.set_policy(Some(LinkLossPolicy::HoldBrief { ticks: 1 }));

        let (t0, s0) = state.tick_link_loss();
        assert_eq!((t0, s0), (1.0, 0.5));
        let (t1, s1) = state.tick_link_loss();
        assert_eq!((t1, s1), (0.0, 0.0));

        // A fresh control frame arrives after the hold window expired but
        // before the link is ever recovered via `set_policy(None)`. The
        // adapter must stay neutralized: `None` is the only path back to
        // normal control per the `VehicleAdapter` trait's documented
        // invariant.
        state.apply(0.9, -0.4);
        let (t2, s2) = state.tick_link_loss();
        assert_eq!((t2, s2), (0.0, 0.0));
        let (t3, s3) = state.tick_link_loss();
        assert_eq!((t3, s3), (0.0, 0.0));
    }

    #[test]
    fn clearing_policy_cancels_in_progress_hold() {
        let mut state = ControlState::default();
        state.apply(1.0, 0.5);
        state.set_policy(Some(LinkLossPolicy::HoldBrief { ticks: 5 }));
        let _ = state.tick_link_loss();

        state.set_policy(None);
        assert_eq!(state.hold_ticks_remaining, None);
        state.apply(0.3, 0.3);
        let (throttle, steering) = state.tick_link_loss();
        assert_eq!((throttle, steering), (0.3, 0.3));
    }
}
