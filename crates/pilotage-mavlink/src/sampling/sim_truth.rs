//! The simulator's ground-truth report as a telemetry sample.
//!
//! Truth is an oracle for judging estimates, so it rides its own source role
//! and never joins the estimate groups. A physical profile must not publish
//! one; the adapter that knows its profile decides whether to attach it.

use pilotage_adapter_api::{
    GeodeticFixSample, MeasurementClock, MeasurementStamp, SimTruthSample, SourceIntegrity,
    SourceRole,
};
use pilotage_geo::{
    BaroSettingId, DatumRealizationId, GeodeticPosition, HorizontalDatum, LocalOriginId,
    PositionQuality, SIMULATOR_GEOID_MODEL_ID, TerrainRefId, VerticalDatum, VerticalPosition,
};
use tracing::warn;

use super::WITHHOLD_AFTER;
use crate::LinkState;
use crate::link::SimTruthUpdate;

/// The simulator's stated position as a typed fix.
///
/// The report gives latitude and longitude in degrees·1e7 and an altitude
/// in millimetres. The message definition names no reference surface for
/// that altitude at all; the senders this lane reads feed it from their
/// own above-mean-sea-level solution, which is why it is labelled MSL here
/// and not in the decoder. An MSL height with no separation is
/// uninterpretable, so the height declares the simulator's own separation:
/// it stays traceable to a simulator and can never be read as a surveyed
/// height. A report the contract refuses is no fix at all — the sample
/// keeps its local frame and declares no position.
fn truth_geodetic_fix(
    truth: &SimTruthUpdate,
    stamp: MeasurementStamp,
) -> Option<GeodeticFixSample> {
    // A report whose whole geodetic triple is zero is a simulator that has
    // not stated a position, not a vehicle at Null Island. The link refuses
    // such a report outright, because the local frame is that position
    // projected; this stays as the contract's own statement at the place
    // that builds the typed value.
    if truth.lat_lon_alt == [0, 0, 0] {
        return None;
    }
    let vertical = VerticalPosition::new(
        f64::from(truth.lat_lon_alt[2]) * 1e-3,
        VerticalDatum::Msl,
        SIMULATOR_GEOID_MODEL_ID,
        TerrainRefId::UNDECLARED,
        BaroSettingId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .inspect_err(|error| {
        warn!(%error, "simulator height refused by the contract; no position published");
    })
    .ok()?;
    let position = GeodeticPosition::new(
        f64::from(truth.lat_lon_alt[0]) * 1e-7,
        f64::from(truth.lat_lon_alt[1]) * 1e-7,
        HorizontalDatum::Wgs84,
        DatumRealizationId::UNDECLARED,
        vertical,
    )
    .inspect_err(|error| {
        warn!(%error, "simulator position refused by the contract; no position published");
    })
    .ok()?;
    Some(GeodeticFixSample {
        position,
        // The simulator states no accuracy. Zero is the honest reading of a
        // value taken from the model rather than measured: it is the
        // oracle, and a reader derives health from the datum identities
        // above rather than from a number the producer invented. A grader
        // that read zero as a metre count would read it as perfect, so
        // this value is safe only while the simulator separation keeps it
        // inside the truth role.
        quality: PositionQuality {
            horizontal_mm: 0,
            vertical_mm: 0,
        },
        stamp,
    })
}

/// The fresh simulation-truth sample, stamped with its own source role so
/// a consumer can never mistake it for the FC's operational estimate.
#[must_use]
pub fn sim_truth_sample(latest: &LinkState) -> Option<SimTruthSample> {
    latest
        .sim_truth
        .filter(|truth| truth.received_at.elapsed() <= WITHHOLD_AFTER)
        .map(|truth| {
            let stamp = MeasurementStamp {
                role: SourceRole::SimulationTruth,
                integrity: SourceIntegrity::ChecksummedOnly,
                source_id: latest.source_id,
                source_incarnation: latest.source_incarnation,
                source_epoch: 0,
                sequence: truth.sequence,
                acquired_at_ns: truth.time_usec.wrapping_mul(1_000),
                clock: MeasurementClock::Simulation,
            };
            SimTruthSample {
                // The same observation the NED group was projected from, so
                // it rides this sample's stamp.
                geodetic: truth_geodetic_fix(&truth, stamp),
                quat_wxyz: truth.quat_wxyz,
                pos_ned_m: truth.pos_ned_m,
                vel_ned_mps: truth.vel_ned_mps,
                // Attitude, position, and velocity are all carried;
                // the truth stream has no body-rate report.
                valid_flags: 0b1101,
                stamp,
            }
        })
}
