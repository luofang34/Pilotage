//! The receiver's own fix, as the typed value a client consumes.
//!
//! Both adapters read the same cached update and build the same value, so
//! it is built once here: two copies of a datum decision drift apart
//! silently, and only the copy some test happens to reach stays correct.

use pilotage_adapter_api::{
    GeodeticFixSample, MeasurementClock, MeasurementStamp, SourceIntegrity, SourceRole,
};
use pilotage_geo::{
    BaroSettingId, DatumRealizationId, GeodeticPosition, HorizontalDatum, LocalOriginId,
    PositionQuality, TerrainRefId, VerticalDatum, VerticalPosition,
};
use tracing::warn;

use super::{GnssFixUpdate, LinkState};

/// The receiver's own fix, as a typed value.
///
/// The height is the one above the WGS-84 ellipsoid, which is
/// interpretable with no geoid model. The sea-level height beside it on
/// the same message names no model for the sea level it means, and there
/// is no honest way to supply that declaration on a receiver's behalf.
///
/// This is a SOLUTION, not an oracle: it rides the estimate role, and a
/// real vehicle reports it the same way a simulated one does.
pub fn estimate_geodetic_fix(fix: &GnssFixUpdate, latest: &LinkState) -> Option<GeodeticFixSample> {
    let vertical = VerticalPosition::new(
        f64::from(fix.alt_ellipsoid_mm) * 1e-3,
        VerticalDatum::Ellipsoid,
        // An ellipsoidal height needs no separation model.
        pilotage_geo::GeoidModelId::UNDECLARED,
        TerrainRefId::UNDECLARED,
        BaroSettingId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .inspect_err(|error| {
        warn!(%error, "receiver height refused by the contract; no position published");
    })
    .ok()?;
    let geodetic = GeodeticPosition::new(
        f64::from(fix.lat_lon[0]) * 1e-7,
        f64::from(fix.lat_lon[1]) * 1e-7,
        HorizontalDatum::Wgs84,
        DatumRealizationId::UNDECLARED,
        vertical,
    )
    .inspect_err(|error| {
        warn!(%error, "receiver position refused by the contract; no position published");
    })
    .ok()?;
    Some(GeodeticFixSample {
        position: geodetic,
        // The receiver states its own 1-sigma accuracy, so this lane
        // carries a measured number rather than a silence.
        quality: PositionQuality {
            horizontal_mm: fix.accuracy_mm[0],
            vertical_mm: fix.accuracy_mm[1],
        },
        // The receiver's own timestamp is not published as an acquisition
        // time: the message states a clock of the sender's choosing and
        // nothing on the wire says which one. Freshness is measured where
        // it can be measured — on the clock that received the report — and
        // ordering comes from the lane's own sequence.
        stamp: MeasurementStamp {
            role: SourceRole::OperationalEstimate,
            integrity: SourceIntegrity::ChecksummedOnly,
            source_id: latest.source_id,
            source_incarnation: latest.source_incarnation,
            source_epoch: latest.source_epoch,
            sequence: fix.sequence,
            acquired_at_ns: fix.received_since_start_ns,
            clock: MeasurementClock::HostMonotonic,
        },
    })
}

#[cfg(test)]
mod tests;
