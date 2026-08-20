//! Pure state machines for control-demand shaping.

use crate::{AxisDynamics, AxisResponse, HoldTransition, NeutralBand};

/// Whether the operator applies or releases a demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemandPhase {
    /// The input is active.
    Apply,
    /// The input is neutral.
    Release,
}

/// Hysteretic classification of one normalized input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NeutralLatch {
    active: bool,
}

impl NeutralLatch {
    /// Update the classification and return whether the input is active.
    pub fn update(&mut self, magnitude: f32, band: NeutralBand) -> bool {
        let magnitude = if magnitude.is_finite() {
            magnitude.abs()
        } else {
            0.0
        };
        self.active = if self.active {
            magnitude > band.active_exit
        } else {
            magnitude > band.active_enter
        };
        self.active
    }

    /// Clear the classification state.
    pub fn reset(&mut self) {
        self.active = false;
    }
}

/// One jerk-limited demand axis.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct JerkLimitedAxis {
    value: f32,
    rate: f32,
}

/// Output from one normalized input axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapedDemand {
    /// Demand in the scaled output unit.
    pub value: f32,
    /// Whether the physical input is in the active hysteresis state.
    pub input_active: bool,
}

/// Static and time-domain shaping for one input axis.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AxisDemandShaper {
    neutral: NeutralLatch,
    demand: JerkLimitedAxis,
}

impl AxisDemandShaper {
    /// Advance one normalized input sample.
    #[must_use]
    pub fn step(
        &mut self,
        normalized: f32,
        scale: f32,
        dt_s: f32,
        response: AxisResponse,
    ) -> ShapedDemand {
        let normalized = if normalized.is_finite() {
            normalized.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let input_active = self.neutral.update(normalized.abs(), response.neutral);
        let target = if input_active {
            response.curve.apply(normalized) * scale
        } else {
            0.0
        };
        let phase = if input_active {
            DemandPhase::Apply
        } else {
            DemandPhase::Release
        };
        ShapedDemand {
            value: self.demand.step(target, dt_s, phase, response.dynamics),
            input_active,
        }
    }

    /// Set the demand without a transient and clear input activity.
    pub fn seed(&mut self, value: f32) {
        self.neutral.reset();
        self.demand.seed(value);
    }

    /// Clear all shaping state.
    pub fn reset(&mut self) {
        self.seed(0.0);
    }

    /// Return the current demand value.
    #[must_use]
    pub fn value(&self) -> f32 {
        self.demand.value()
    }
}

impl JerkLimitedAxis {
    /// Advance the demand by one time step.
    #[must_use]
    pub fn step(
        &mut self,
        target: f32,
        dt_s: f32,
        phase: DemandPhase,
        limits: AxisDynamics,
    ) -> f32 {
        if !target.is_finite() || !dt_s.is_finite() || dt_s <= 0.0 {
            return self.value;
        }
        let target = target.clamp(-100.0, 100.0);
        let (accel, jerk) = match phase {
            DemandPhase::Apply => (limits.apply_accel, limits.apply_jerk),
            DemandPhase::Release => (limits.release_accel, limits.release_jerk),
        };
        let error = target - self.value;
        let step_jerk = jerk * dt_s;
        let stopping_rate =
            ((step_jerk * step_jerk + 2.0 * jerk * error.abs()).sqrt() - step_jerk).max(0.0);
        let desired_rate = error.signum() * stopping_rate.min(accel);
        let rate_delta = (desired_rate - self.rate).clamp(-jerk * dt_s, jerk * dt_s);
        self.rate = (self.rate + rate_delta).clamp(-accel, accel);
        let candidate = self.value + self.rate * dt_s;
        if crossed_target(self.value, candidate, target) {
            self.value = target;
            self.rate = 0.0;
        } else {
            self.value = candidate;
        }
        self.value
    }

    /// Return the current demand value.
    #[must_use]
    pub fn value(&self) -> f32 {
        self.value
    }

    /// Return the current first derivative.
    #[must_use]
    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Set the state without a transient.
    pub fn seed(&mut self, value: f32) {
        self.value = if value.is_finite() { value } else { 0.0 };
        self.rate = 0.0;
    }

    /// Clear the state.
    pub fn reset(&mut self) {
        self.seed(0.0);
    }
}

/// Stable-dwell detector for a brake-to-hold transition.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HoldDetector {
    stable_s: f32,
}

impl HoldDetector {
    /// Update the stable interval and report if capture is permitted.
    pub fn update(
        &mut self,
        speed_mps: Option<f32>,
        accel_mps2: Option<f32>,
        dt_s: f32,
        policy: HoldTransition,
    ) -> bool {
        let accel_stable = if policy.require_accel {
            finite_within(accel_mps2, policy.max_accel_mps2)
        } else {
            accel_mps2.is_none_or(|value| value.is_finite() && value.abs() <= policy.max_accel_mps2)
        };
        let stable_sample = finite_within(speed_mps, policy.max_speed_mps) && accel_stable;
        if !stable_sample || !dt_s.is_finite() || dt_s < 0.0 {
            self.stable_s = 0.0;
            return false;
        }
        self.stable_s += dt_s;
        self.stable_s * 1_000.0 >= policy.stable_dwell_ms as f32
    }

    /// Clear the stable interval.
    pub fn reset(&mut self) {
        self.stable_s = 0.0;
    }
}

fn finite_within(value: Option<f32>, limit: f32) -> bool {
    value.is_some_and(|value| value.is_finite() && value.abs() <= limit)
}

fn crossed_target(before: f32, after: f32, target: f32) -> bool {
    (before <= target && after >= target) || (before >= target && after <= target)
}
