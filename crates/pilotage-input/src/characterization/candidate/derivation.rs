//! Deterministic values that promotion can derive from axis evidence.

use super::AxisCharacterization;
use crate::{AxisCalibration, DeadzoneEvidenceStatus, NeutralPosition};

const MIN_EXCURSION: f32 = 1.0e-4;
const MIN_DEADZONE: f32 = 0.002;
const MAX_DEADZONE: f32 = 0.2;
const MIN_RANGE_COVERAGE: f32 = 0.98;

impl AxisCharacterization {
    /// Derives the only calibration that matches the observed values.
    #[must_use]
    pub fn derived_calibration(&self) -> AxisCalibration {
        let span = (self.observed_max - self.observed_min).max(MIN_EXCURSION);
        let padding = (span * 1.0e-6).max(f32::EPSILON * self.observed_center.abs().max(1.0));
        let lower = (self.observed_center - self.observed_min).max(padding);
        let upper = (self.observed_max - self.observed_center).max(padding);
        match self.source_range.neutral_position {
            NeutralPosition::Centered => AxisCalibration {
                min: self.observed_center - lower,
                center: self.observed_center,
                max: self.observed_center + upper,
            },
            NeutralPosition::Minimum => AxisCalibration {
                min: self.observed_center - upper,
                center: self.observed_center,
                max: self.observed_center + upper,
            },
            NeutralPosition::Maximum => AxisCalibration {
                min: self.observed_center - lower,
                center: self.observed_center,
                max: self.observed_center + lower,
            },
        }
    }

    /// Derives inversion from the first physical-direction excursion.
    #[must_use]
    pub fn derived_invert(&self) -> bool {
        self.first_direction_excursion.is_sign_negative()
    }

    /// Calculates movement coverage against the trusted source range.
    #[must_use]
    pub fn range_coverage(&self) -> f32 {
        let expected_lower = self.observed_center - self.source_range.minimum;
        let expected_upper = self.source_range.maximum - self.observed_center;
        if expected_lower < 0.0 || expected_upper < 0.0 {
            return 0.0;
        }
        let lower = ratio(self.observed_center - self.observed_min, expected_lower);
        let upper = ratio(self.observed_max - self.observed_center, expected_upper);
        match self.source_range.neutral_position {
            NeutralPosition::Centered => lower.min(upper),
            NeutralPosition::Minimum => upper,
            NeutralPosition::Maximum => lower,
        }
    }

    /// Reports whether movement reached the required source range.
    #[must_use]
    pub fn has_required_range(&self) -> bool {
        self.range_coverage() >= MIN_RANGE_COVERAGE
    }

    /// Derives the only device-noise dead zone supported by the evidence.
    #[must_use]
    pub fn derived_deadzone(&self, status: DeadzoneEvidenceStatus) -> f32 {
        if status != DeadzoneEvidenceStatus::NotObserved {
            return 0.0;
        }
        let idle_seconds = self.idle_duration_us as f32 / 1_000_000.0;
        let disturbance =
            (self.center_noise * 4.0).max(self.center_drift_per_second * idle_seconds);
        let calibration = self.derived_calibration();
        let lower = calibration.center - calibration.min;
        let upper = calibration.max - calibration.center;
        let reachable_span = match self.source_range.neutral_position {
            NeutralPosition::Centered => lower.min(upper),
            NeutralPosition::Minimum => upper,
            NeutralPosition::Maximum => lower,
        }
        .max(MIN_EXCURSION);
        let normalized = disturbance / reachable_span;
        if normalized < MIN_DEADZONE {
            0.0
        } else {
            normalized.clamp(MIN_DEADZONE, MAX_DEADZONE)
        }
    }

    /// Derives confidence from sample count, uniqueness, range, and timing.
    #[must_use]
    pub fn derived_confidence(&self, timing_confidence: f32) -> f32 {
        let evidence_count = self.idle_sample_count.min(self.movement_sample_count);
        let sample_confidence = (evidence_count as f32 / 12.0).clamp(0.0, 1.0);
        let uniqueness = (1.0 - self.cross_axis_coupling).clamp(0.0, 1.0);
        timing_confidence.min(sample_confidence * uniqueness * self.range_coverage())
    }
}

fn ratio(observed: f32, expected: f32) -> f32 {
    if expected <= MIN_EXCURSION {
        1.0
    } else {
        (observed / expected).clamp(0.0, 1.0)
    }
}
