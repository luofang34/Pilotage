//! Telemetry from the latest poll: the same sample shape the PX4 adapter
//! publishes, built from Betaflight's estimate.
//!
//! Attitude and rates are valid whenever a poll is fresh. Position and
//! velocity are valid only with a usable 3-D fix: horizontal position is
//! the fix measured from the attachment's anchor, horizontal velocity is
//! the receiver's ground velocity, and the vertical axis is Betaflight's
//! fused altitude and climb rate.

use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, FcStateSample,
    GeodeticFixSample, MeasurementClock, MeasurementStamp, Pose2d, SourceIntegrity, SourceRole,
    TelemetryBatch, TelemetrySample,
};
use pilotage_geo::{
    BaroSettingId, DatumRealizationId, GeodeticPosition, GeoidModelId, HorizontalDatum,
    LocalOriginId, PositionQuality, TerrainRefId, VerticalDatum, VerticalPosition,
};

use super::BetaflightAdapter;
use crate::link::Observation;

/// Authorization bits: attitude, rates, position, velocity.
const VALID_ATTITUDE_RATES: u32 = 0b0011;
const VALID_POSITION_VELOCITY: u32 = 0b1100;
/// Estimate quality: good.
const QUALITY_GOOD: u32 = 0;
/// Source id of the estimate lane; the FC-state lane has its own.
const ESTIMATE_SOURCE_ID: u64 = 1;
const FC_STATE_SOURCE_ID: u64 = 2;

impl BetaflightAdapter {
    pub(super) fn telemetry_batch(&self) -> TelemetryBatch {
        let Some(observed) = self.fresh_observation() else {
            return TelemetryBatch::default();
        };
        let last_command = self.reports.borrow().last_command;
        let estimate_stamp = self.stamp(
            &observed,
            SourceRole::OperationalEstimate,
            ESTIMATE_SOURCE_ID,
        );
        let snapshot = &observed.snapshot;
        let kinematics =
            observed
                .north_east
                .zip(snapshot.gnss)
                .map(|(ne, fix)| AvionicsKinematicsSample {
                    pos_ned_m: [ne[0] as f32, ne[1] as f32, -snapshot.altitude_m as f32],
                    vel_ned_mps: [
                        fix.velocity_ne[0] as f32,
                        fix.velocity_ne[1] as f32,
                        -snapshot.climb_rate as f32,
                    ],
                    stamp: estimate_stamp,
                });
        let valid_flags = VALID_ATTITUDE_RATES
            | if kinematics.is_some() {
                VALID_POSITION_VELOCITY
            } else {
                0
            };
        let avionics = AvionicsSample {
            attitude: Some(AvionicsAttitudeSample::Canonical {
                quat_wxyz: snapshot.quaternion().map(|q| q as f32),
                rates_rps: Some(snapshot.rates_frd.map(|r| r as f32)),
                stamp: estimate_stamp,
            }),
            kinematics,
            baro: None,
            geodetic: self.geodetic_fix(&observed, estimate_stamp),
            estimator_status_stamp: Some(estimate_stamp),
            valid_flags,
            quality: QUALITY_GOOD,
            magnetic_variation: None,
        };
        let fc_state = FcStateSample {
            landed_state: None,
            motor_state: None,
            arm_state: if snapshot.armed { 2 } else { 1 },
            last_command,
            stamp: Some(self.stamp(&observed, SourceRole::FcState, FC_STATE_SOURCE_ID)),
        };
        let pose = observed.north_east.map(|[north, east]| Pose2d {
            x: north,
            y: east,
            heading: snapshot.heading,
        });
        let speed = snapshot
            .gnss
            .map(|fix| fix.velocity_ne[0].hypot(fix.velocity_ne[1]));
        TelemetryBatch {
            samples: vec![TelemetrySample {
                flight_procedure: None,
                payload: None,
                vehicle: self.vehicle,
                tick: self.tick_of(snapshot.received_at),
                pose,
                speed,
                avionics: Some(avionics),
                sim_truth: None,
                fc_state: Some(fc_state),
                gimbal: None,
                battery: None,
            }],
        }
    }

    fn stamp(&self, observed: &Observation, role: SourceRole, source_id: u64) -> MeasurementStamp {
        let since = observed
            .snapshot
            .received_at
            .saturating_duration_since(self.started_at);
        MeasurementStamp {
            role,
            // MSP frames carry a checksum and no authentication.
            integrity: SourceIntegrity::ChecksummedOnly,
            source_id,
            source_incarnation: self.incarnation,
            // A reconnect is a new epoch: the sequence restarts with it.
            source_epoch: observed.snapshot.epoch,
            sequence: observed.snapshot.sequence,
            acquired_at_ns: u64::try_from(since.as_nanos()).unwrap_or(u64::MAX),
            clock: MeasurementClock::HostMonotonic,
        }
    }

    /// The fix as a geodetic position. Betaflight's GNSS altitude is a
    /// whole-metre sea-level height with no declared geoid, which the
    /// contract refuses; its fused altitude above the attachment's anchor is
    /// a declared local-relative height instead.
    fn geodetic_fix(
        &self,
        observed: &Observation,
        stamp: MeasurementStamp,
    ) -> Option<GeodeticFixSample> {
        let fix = observed.snapshot.gnss?;
        let vertical = VerticalPosition::new(
            observed.snapshot.altitude_m,
            VerticalDatum::LocalRelative,
            GeoidModelId::UNDECLARED,
            TerrainRefId::UNDECLARED,
            BaroSettingId::UNDECLARED,
            self.local_origin(observed),
        )
        .inspect_err(|error| tracing::warn!(%error, "Betaflight height refused; no fix published"))
        .ok()?;
        let position = GeodeticPosition::new(
            fix.lat_deg,
            fix.lon_deg,
            HorizontalDatum::Wgs84,
            DatumRealizationId::UNDECLARED,
            vertical,
        )
        .inspect_err(|error| tracing::warn!(%error, "Betaflight fix refused; no fix published"))
        .ok()?;
        Some(GeodeticFixSample {
            position,
            // MSP_RAW_GPS states dilution, not a 1-sigma accuracy.
            quality: PositionQuality {
                horizontal_mm: 0,
                vertical_mm: 0,
            },
            stamp,
        })
    }

    /// The local-relative origin's identity: this attachment and link
    /// epoch, so a new anchor is a visible identity change.
    fn local_origin(&self, observed: &Observation) -> LocalOriginId {
        let bytes = self.incarnation.into_bytes();
        let mut id = [0u8; 8];
        id.copy_from_slice(&bytes[..8]);
        let id = u64::from_le_bytes(id) ^ u64::from(observed.snapshot.epoch) | 1;
        LocalOriginId(id)
    }
}
