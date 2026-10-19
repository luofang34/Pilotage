//! The flight controller's geomagnetic reference, as the typed variation
//! a client consumes.

use pilotage_adapter_api::{
    MagneticVariationSample, MagneticVariationSource, MeasurementClock, MeasurementStamp,
    SourceIntegrity, SourceRole,
};

use super::{GeomagneticReferenceUpdate, LinkState};

/// The variation an FC geomagnetic reference report states, or `None` for
/// a source code this build does not know.
///
/// The report names no model or epoch, so the sample states none. Like the
/// receiver fix, it is stamped on the receiving clock: the report's own
/// timestamp names a clock of the sender's choosing.
pub fn fc_magnetic_variation(
    update: &GeomagneticReferenceUpdate,
    latest: &LinkState,
) -> Option<MagneticVariationSample> {
    let source = fc_variation_source(update.source)?;
    Some(MagneticVariationSample {
        declination_rad: update.declination_rad,
        source,
        model_id: None,
        model_epoch: None,
        stamp: MeasurementStamp {
            role: SourceRole::OperationalEstimate,
            integrity: SourceIntegrity::ChecksummedOnly,
            source_id: latest.source_id,
            source_incarnation: latest.source_incarnation,
            source_epoch: latest.source_epoch,
            sequence: update.sequence,
            acquired_at_ns: update.received_since_start_ns,
            clock: MeasurementClock::HostMonotonic,
        },
    })
}

/// The variation source a geomagnetic reference report's source code names,
/// or `None` for a code this build does not know.
pub fn fc_variation_source(code: u8) -> Option<MagneticVariationSource> {
    match code {
        0 => Some(MagneticVariationSource::None),
        1 => Some(MagneticVariationSource::FcModel),
        2 => Some(MagneticVariationSource::FcConfigured),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
