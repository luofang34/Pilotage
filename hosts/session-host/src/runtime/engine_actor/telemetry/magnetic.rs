//! A vehicle whose adapter declares a magnetic heading reference and
//! states no variation of its own gets the host's: its authorized attitude
//! turns to true north by the shared geomagnetic model's declination at its
//! own fix, and the variation applied travels in the same sample. Every
//! consumer, the mission estimate and the clients alike, then reads a true
//! heading and sees what turned it (ADR-0051).

use std::time::{SystemTime, UNIX_EPOCH};

use pilotage_adapter_api::{
    AvionicsAttitudeSample, MagneticVariationSample, MagneticVariationSource, MeasurementStamp,
    SourceRole, TelemetrySample,
};
use pilotage_geo::VerticalDatum;
use pilotage_protocol::HeadingReference;
use pilotage_world::{MagneticEpoch, WMM2025};
use tracing::{info, warn};

#[cfg(test)]
mod tests;

/// Turns the magnetic headings of the vehicles that declare them true.
#[derive(Debug)]
pub(in crate::runtime::engine_actor) struct MagneticCorrection {
    /// The one date the field is evaluated at for the whole run.
    epoch: MagneticEpoch,
    announced: bool,
    refused: bool,
}

impl MagneticCorrection {
    /// At the host's civil time when it starts.
    pub(in crate::runtime::engine_actor) fn at_host_time() -> Self {
        let unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
            });
        Self::at(MagneticEpoch::from_unix_seconds(unix_seconds))
    }

    /// At a given epoch.
    pub(in crate::runtime::engine_actor) fn at(epoch: MagneticEpoch) -> Self {
        Self {
            epoch,
            announced: false,
            refused: false,
        }
    }

    /// For a vehicle whose adapter declared a magnetic heading reference:
    /// turns the sample's authorized attitude to true north by the model's
    /// declination at the sample's own fix and states the variation
    /// applied. A sample without an authorized attitude or a fix, or whose
    /// vehicle states its own variation, passes unchanged. Returns the
    /// declination applied, radians, positive east.
    pub(in crate::runtime::engine_actor) fn apply(
        &mut self,
        sample: &mut TelemetrySample,
        reference: Option<HeadingReference>,
    ) -> Option<f32> {
        if reference != Some(HeadingReference::Magnetic) {
            return None;
        }
        let avionics = sample.avionics.as_mut()?;
        if avionics.magnetic_variation.is_some() {
            return None;
        }
        let position = avionics.geodetic.as_ref()?.position;
        let Some(AvionicsAttitudeSample::Canonical {
            quat_wxyz, stamp, ..
        }) = avionics.attitude.as_mut()
        else {
            return None;
        };
        // The declination changes by less than the model's own uncertainty
        // over any flight height, so a height the fix measures from the
        // launch point or from the ground is taken at sea level.
        let height_m = match position.vertical.datum {
            VerticalDatum::Msl | VerticalDatum::Ellipsoid => position.vertical.height_m,
            _ => 0.0,
        };
        let field = match pilotage_world::magnetic_field_at(
            position.latitude_deg,
            position.longitude_deg,
            height_m,
            self.epoch,
        ) {
            Ok(field) => field,
            Err(error) => {
                if !self.refused {
                    self.refused = true;
                    warn!(%error, "magnetic heading left as reported: the shared model refused the fix");
                }
                return None;
            }
        };
        let declination_rad = field.declination_rad() as f32;
        *quat_wxyz = turned_about_down(*quat_wxyz, declination_rad);
        let variation_stamp = MeasurementStamp {
            role: SourceRole::OperationalEstimate,
            ..*stamp
        };
        avionics.magnetic_variation = Some(MagneticVariationSample {
            declination_rad,
            source: MagneticVariationSource::SharedModel,
            model_id: Some(WMM2025.dataset.name),
            model_epoch: Some(WMM2025.epoch.0),
            stamp: variation_stamp,
        });
        if !self.announced {
            self.announced = true;
            info!(
                vehicle = ?sample.vehicle,
                declination_deg = declination_rad.to_degrees(),
                model = WMM2025.dataset.name,
                "magnetic heading turned to true north by the shared model"
            );
        }
        Some(declination_rad)
    }
}

/// `[w, x, y, z]` turned by `angle` about the down axis: a heading moved
/// clockwise by `angle`, roll and pitch untouched.
fn turned_about_down([w, x, y, z]: [f32; 4], angle: f32) -> [f32; 4] {
    let (sin, cos) = (angle / 2.0).sin_cos();
    [
        cos * w - sin * z,
        cos * x - sin * y,
        cos * y + sin * x,
        cos * z + sin * w,
    ]
}
