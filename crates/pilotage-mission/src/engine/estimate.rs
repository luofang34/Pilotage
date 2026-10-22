//! The vehicle's own estimate as a pass-through navigation solution.
//!
//! A flight controller's exported state is estimator-derived (ADR-0024):
//! the fusion filter refuses it as an aid, and the engine does not present
//! it as a receiver fix. In the vehicle-estimate mode the engine flies the
//! latest fresh sample directly. The solution declares its composition as
//! FC state, carries the configured uncertainty, and never grades better
//! than degraded: one estimator, no redundancy, no fault detection.

use navigate_contract::{
    ClockDomainId, DurationNanos, FaultDetection, IntegrityAssessment, MonotonicNanos,
    NavigationSolution, NedVelocity, Redundancy, SensorClass, SolutionQuality, SolutionStamp,
    SourceComposition, SourceEpoch, SymmetricCov3, WrappingSequence,
};
use navigate_geodesy::{LocalTangentPlane, NedOffset};

use crate::ownship::OwnshipSample;

/// The uncertainty the pass-through solution declares.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct EstimateBudget {
    /// 1-sigma position uncertainty per NED axis, meters.
    pub(super) sigma_m: f64,
    /// 1-sigma velocity uncertainty per NED axis, meters per second.
    pub(super) velocity_sigma_mps: f64,
    /// Age past which the latest sample yields no solution.
    pub(super) max_age: DurationNanos,
}

/// The latest admitted estimate sample.
#[derive(Debug, Default)]
pub(super) struct VehicleEstimate {
    latest: Option<OwnshipSample>,
}

impl VehicleEstimate {
    /// Keep the sample as the current estimate.
    pub(super) fn accept(&mut self, sample: &OwnshipSample) {
        self.latest = Some(*sample);
    }

    /// Whether a sample has ever been admitted.
    pub(super) fn has_sample(&self) -> bool {
        self.latest.is_some()
    }

    /// The latest sample as a solution for `now`, or `None` when no
    /// sample is admitted, the sample is older than the budget allows,
    /// or the sample is stamped after `now`.
    pub(super) fn solution(
        &self,
        now: MonotonicNanos,
        plane: &LocalTangentPlane,
        clock: ClockDomainId,
        budget: EstimateBudget,
    ) -> Option<NavigationSolution> {
        let sample = self.latest?;
        let age = now.elapsed_since(sample.acquired_at)?;
        if age.as_nanos() > budget.max_age.as_nanos() {
            return None;
        }
        let position = plane.from_ned(&NedOffset::new(sample.ned[0], sample.ned[1], sample.ned[2]));
        let velocity = NedVelocity::new(
            sample.ned_velocity[0],
            sample.ned_velocity[1],
            sample.ned_velocity[2],
        );
        let position_variance = budget.sigma_m * budget.sigma_m;
        let velocity_variance = budget.velocity_sigma_mps * budget.velocity_sigma_mps;
        let composition = SourceComposition::of(SensorClass::FcState);
        let integrity = IntegrityAssessment::new(
            SolutionQuality::Degraded,
            composition,
            Redundancy::None,
            budget.sigma_m,
            budget.sigma_m,
            FaultDetection::Unavailable,
        );
        let stamp = SolutionStamp::new(
            SourceEpoch::new(0),
            WrappingSequence::new(sample.sequence),
            sample.acquired_at,
            clock,
        );
        Some(NavigationSolution::new(
            stamp,
            position,
            velocity,
            SymmetricCov3::from_diagonal(position_variance, position_variance, position_variance),
            SymmetricCov3::from_diagonal(velocity_variance, velocity_variance, velocity_variance),
            integrity,
            composition,
        ))
    }
}
