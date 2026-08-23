//! Pure state machines for control-demand shaping.

use crate::{AxisDynamics, AxisResponse, HoldTransition, NeutralBand};

const TARGET_EPSILON: f32 = 1.0e-6;

/// Whether the operator applies or releases a demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemandPhase {
    /// The input is active.
    Apply,
    /// The input is neutral.
    Release,
    /// The input requests the opposite direction.
    Reversal,
}

/// Hysteretic classification of one normalized input.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NeutralLatch {
    active: bool,
    neutral_s: f32,
}

impl NeutralLatch {
    /// Update the classification and return whether the input is active.
    pub fn update(&mut self, magnitude: f32, dt_s: f32, band: NeutralBand) -> bool {
        let magnitude = if magnitude.is_finite() {
            magnitude.abs()
        } else {
            0.0
        };
        if !self.active {
            self.active = magnitude > band.active_enter;
            self.neutral_s = 0.0;
            return self.active;
        }
        if magnitude > band.active_exit {
            self.neutral_s = 0.0;
            return true;
        }
        if !dt_s.is_finite() || dt_s < 0.0 {
            return true;
        }
        self.neutral_s += dt_s;
        if self.neutral_s * 1_000.0 >= band.dwell_ms as f32 {
            self.active = false;
            self.neutral_s = 0.0;
        }
        self.active
    }

    /// Clear the classification state.
    pub fn reset(&mut self) {
        self.active = false;
        self.neutral_s = 0.0;
    }
}

/// One jerk-limited demand axis.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct JerkLimitedAxis {
    value: f32,
    rate: f32,
    phase: Option<DemandPhase>,
}

/// Output from one normalized input axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapedDemand {
    /// Demand in the scaled output unit.
    pub value: f32,
    /// Whether the physical input is in the active hysteresis state.
    pub input_active: bool,
    /// Time-domain state used for this sample.
    pub phase: DemandPhase,
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
        let curved = response.curve.apply(normalized);
        let input_active = self.neutral.update(curved.abs(), dt_s, response.neutral);
        let target = if input_active { curved * scale } else { 0.0 };
        let phase = demand_phase(
            input_active,
            target,
            self.demand.value(),
            self.demand.rate(),
        );
        ShapedDemand {
            value: self.demand.step(target, dt_s, phase, response.dynamics),
            input_active,
            phase,
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
        if !target.is_finite() || !dt_s.is_finite() || dt_s <= 0.0 || !limits_are_usable(limits) {
            return self.value;
        }
        let target = target.clamp(-100.0, 100.0);
        let (accel, jerk) = match phase {
            DemandPhase::Apply => (limits.apply_accel, limits.apply_jerk),
            DemandPhase::Release => (limits.release_accel, limits.release_jerk),
            DemandPhase::Reversal => (limits.reversal_accel, limits.reversal_jerk),
        };
        let error = target - self.value;
        let step_jerk = jerk * dt_s;
        if error.abs() <= TARGET_EPSILON && self.rate.abs() <= step_jerk {
            self.value = target;
            self.rate = 0.0;
            self.phase = Some(phase);
            return self.value;
        }
        let stopping_rate =
            ((step_jerk * step_jerk + 2.0 * jerk * error.abs()).sqrt() - step_jerk).max(0.0);
        let desired_rate = if error.abs() <= TARGET_EPSILON {
            0.0
        } else {
            error.signum() * stopping_rate.min(accel)
        };
        let rate_delta = (desired_rate - self.rate).clamp(-jerk * dt_s, jerk * dt_s);
        self.rate += rate_delta;
        let next = self.value + self.rate * dt_s;
        let crossed_target = error != 0.0 && (target - next).signum() != error.signum();
        if crossed_target && self.rate.abs() <= step_jerk {
            self.value = target;
            self.rate = 0.0;
        } else {
            self.value = next;
        }
        self.phase = Some(phase);
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
        self.phase = None;
    }

    /// Clear the state.
    pub fn reset(&mut self) {
        self.seed(0.0);
    }
}

fn limits_are_usable(limits: AxisDynamics) -> bool {
    [
        limits.apply_accel,
        limits.release_accel,
        limits.apply_jerk,
        limits.release_jerk,
        limits.reversal_accel,
        limits.reversal_jerk,
    ]
    .into_iter()
    .all(|value| value.is_finite() && value > 0.0)
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

fn demand_phase(input_active: bool, target: f32, value: f32, rate: f32) -> DemandPhase {
    if !input_active {
        return DemandPhase::Release;
    }
    let direction = if value.abs() > TARGET_EPSILON {
        value.signum()
    } else if rate.abs() > TARGET_EPSILON {
        rate.signum()
    } else {
        target.signum()
    };
    if target != 0.0 && direction != target.signum() {
        DemandPhase::Reversal
    } else {
        DemandPhase::Apply
    }
}
