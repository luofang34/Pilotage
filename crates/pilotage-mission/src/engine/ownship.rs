//! Ownship intake: which samples the configured source admits, and the
//! navigation solution each source gives a tick.

use navigate_contract::{
    GeodeticPosition, MonotonicNanos, NavigationSolution, ObservationStamp, SensorClass,
    SourceComposition, SourceEpoch, SourceId, SymmetricCov3, WrappingSequence,
};
use navigate_fusion::{Observation, ObservationValue};
use navigate_geodesy::NedOffset;

use crate::config::OwnshipSource;
use crate::ownship::{OwnshipAltitude, OwnshipSample, TruthRole};

use super::MissionEngine;

/// The single synthesized-GNSS source identity the engine stamps its
/// observations with.
const OWNSHIP_SOURCE: SourceId = SourceId::new(1);

impl MissionEngine {
    /// The solution the engine published at its last tick: the one it
    /// flies, or `None` when it has none to fly.
    #[must_use]
    pub fn solution(&self) -> Option<&NavigationSolution> {
        self.last_solution.as_ref()
    }

    /// Offers one ownship sample. The configured [`OwnshipSource`] names
    /// the one role the engine admits — see [`OwnshipSample`] for why any
    /// other role is a counted refusal, never an aid. Under
    /// [`OwnshipSource::OnboardNavigation`] no role is admitted: ownship
    /// comes through [`Self::on_onboard_solution`]. `now` is the
    /// admission reference on the engine's clock domain.
    pub fn on_ownship(&mut self, sample: &OwnshipSample, now: MonotonicNanos) {
        let admitted = match self.config.ownship_source {
            OwnshipSource::SimulationTruth => Some(TruthRole::SimulationTruth),
            OwnshipSource::VehicleEstimate { .. } => Some(TruthRole::OperationalEstimate),
            OwnshipSource::OnboardNavigation { .. } => None,
        };
        if admitted != Some(sample.role) {
            self.counters.rejected_role = self.counters.rejected_role.wrapping_add(1);
            return;
        }
        self.last_yaw_rad = sample.yaw_rad;
        if sample.role == TruthRole::OperationalEstimate {
            self.estimate.accept(sample);
            return;
        }
        self.truth_sampled = true;
        self.fuse_truth(sample, now);
    }

    /// NED meters from the mission anchor of a geodetic reading, for a
    /// host that converts the vehicle's own fix into an ownship sample.
    /// An above-anchor height is measured from the anchor altitude.
    #[must_use]
    pub fn ned_of(
        &self,
        latitude_rad: f64,
        longitude_rad: f64,
        altitude: OwnshipAltitude,
    ) -> [f64; 3] {
        let altitude_m = match altitude {
            OwnshipAltitude::Ellipsoid(altitude_m) => altitude_m,
            OwnshipAltitude::AboveAnchor(height_m) => self.config.anchor.altitude_m + height_m,
        };
        let offset = self.plane.to_ned(&GeodeticPosition::new(
            latitude_rad,
            longitude_rad,
            altitude_m,
        ));
        [offset.north_m, offset.east_m, offset.down_m]
    }

    /// Synthesizes a GNSS-class fix from a simulation-truth sample and
    /// offers it to the fusion filter.
    fn fuse_truth(&mut self, sample: &OwnshipSample, now: MonotonicNanos) {
        let position =
            self.plane
                .from_ned(&NedOffset::new(sample.ned[0], sample.ned[1], sample.ned[2]));
        let variance = self.config.gnss_sigma_m * self.config.gnss_sigma_m;
        let stamp = ObservationStamp::new(
            OWNSHIP_SOURCE,
            // Epoch stays 0 until a reset/relaunch story exists; the
            // filter treats a new epoch as a new source incarnation.
            SourceEpoch::new(0),
            WrappingSequence::new(sample.sequence),
            sample.acquired_at,
            self.config.clock,
        );
        let observation = Observation::new(
            stamp,
            ObservationValue::PositionFix {
                position,
                covariance: SymmetricCov3::from_diagonal(variance, variance, variance),
            },
            SourceComposition::of(SensorClass::Gnss),
        );
        if !self.filter.ingest(&observation, now).is_accepted() {
            self.counters.fusion_rejected = self.counters.fusion_rejected.wrapping_add(1);
        }
    }
}
