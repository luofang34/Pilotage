//! Advisory shape display policy.

use airmass_core::{
    AdvisoryAltitude, AdvisoryAltitudeBand, AdvisoryAltitudeReference, WeatherAdvisory,
    WeatherAdvisoryType,
};
use airmass_geojson::{AdvisoryShapeFeature, AdvisoryShapeRing};

use crate::layer::WEATHER_ADVISORY_LAYER_ID;
use crate::policy::{
    ADVISORY_AIRMET_STYLE, ADVISORY_CONVECTIVE_STYLE, ADVISORY_CWA_STYLE, ADVISORY_G_AIRMET_STYLE,
    ADVISORY_SIGMET_STYLE,
};
use crate::{Coordinate, CoordinateRing, ShapeFeature};

const FEET_TO_METRES: f64 = 0.3048;

/// Least thickness one advisory volume draws with, in metres.
const MINIMUM_VOLUME_THICKNESS_M: f64 = 150.0;

/// Convert one advisory shape feature into a display shape.
///
/// A polyline advisory returns nothing. The display shape carries polygon rings, and a
/// line drawn as a ring would claim an enclosed area the advisory never stated.
pub(crate) fn shape_for_advisory(
    id: String,
    feature: &AdvisoryShapeFeature,
) -> Option<ShapeFeature> {
    let rings: Vec<CoordinateRing> = feature.rings().iter().filter_map(ring).collect();
    if rings.is_empty() {
        return None;
    }
    let advisory = feature.advisory();
    let (base_above_terrain_m, top_above_terrain_m) = extrusion(advisory);
    Some(ShapeFeature {
        id,
        layer_id: WEATHER_ADVISORY_LAYER_ID.into(),
        rings,
        style_id: style_for(advisory_type(advisory)).into(),
        label: label(advisory),
        base_above_terrain_m,
        top_above_terrain_m,
        producer_instance_id: feature.id().producer_instance_id().get(),
        snapshot_revision: feature.revisions().snapshot_revision().get(),
    })
}

/// Turn one altitude band into the heights the renderer raises the outline between.
///
/// An advisory without a band stays flat. A volume drawn from an invented band would
/// state an altitude range the advisory never gave.
fn extrusion(advisory: &WeatherAdvisory) -> (Option<f64>, Option<f64>) {
    let Some(band) = advisory
        .altitude_band()
        .as_present()
        .map(|field| field.value)
    else {
        return (None, None);
    };
    let base = f64::from(band.base().feet()) * FEET_TO_METRES;
    let top = f64::from(band.top().feet()) * FEET_TO_METRES;
    // A band whose limits are equal draws nothing, and a surface-to-surface advisory is
    // still an area a pilot must see.
    if top - base < MINIMUM_VOLUME_THICKNESS_M {
        return (Some(base), Some(base + MINIMUM_VOLUME_THICKNESS_M));
    }
    (Some(base), Some(top))
}

fn ring(source: &AdvisoryShapeRing) -> Option<CoordinateRing> {
    let mut coordinates: Vec<Coordinate> = source
        .positions()
        .iter()
        .filter_map(|position| {
            Coordinate::checked(position.latitude_deg(), position.longitude_deg())
        })
        .collect();
    // A ring that lost a position to the range check is no longer the stated outline, so
    // the whole ring goes rather than a silently different shape.
    if coordinates.len() != source.positions().len() || coordinates.len() < 3 {
        return None;
    }
    if coordinates.first() != coordinates.last() {
        let first = coordinates[0];
        coordinates.push(first);
    }
    Some(CoordinateRing { coordinates })
}

const fn advisory_type(advisory: &WeatherAdvisory) -> Option<WeatherAdvisoryType> {
    match advisory.advisory_type().as_present() {
        Some(field) => Some(field.value),
        None => None,
    }
}

const fn style_for(advisory_type: Option<WeatherAdvisoryType>) -> &'static str {
    match advisory_type {
        Some(WeatherAdvisoryType::ConvectiveSigmet) => ADVISORY_CONVECTIVE_STYLE,
        Some(WeatherAdvisoryType::GAirmet) => ADVISORY_G_AIRMET_STYLE,
        Some(WeatherAdvisoryType::CenterWeatherAdvisory) => ADVISORY_CWA_STYLE,
        Some(WeatherAdvisoryType::Airmet) => ADVISORY_AIRMET_STYLE,
        _ => ADVISORY_SIGMET_STYLE,
    }
}

/// Name the advisory and state the altitude it covers.
///
/// An outline alone cannot answer "does this affect my cruise altitude", so the band
/// travels with the label until the renderer draws the volume.
fn label(advisory: &WeatherAdvisory) -> Option<String> {
    let name = type_name(advisory_type(advisory)?);
    Some(match band(advisory) {
        Some(band) => format!("{name} {band}"),
        None => name.to_owned(),
    })
}

fn band(advisory: &WeatherAdvisory) -> Option<String> {
    let band: AdvisoryAltitudeBand = advisory.altitude_band().as_present()?.value;
    Some(format!(
        "{}-{}",
        altitude(band.base()),
        altitude(band.top())
    ))
}

/// Name one limit in the vocabulary its reference belongs to.
///
/// A limit printed in the wrong reference reads as a different altitude: a flight level
/// shown as a mean-sea-level height, or a surface limit shown as zero feet, both state a
/// height the advisory did not.
fn altitude(value: AdvisoryAltitude) -> String {
    let feet = value.feet();
    match value.reference() {
        AdvisoryAltitudeReference::MeanSeaLevel => format!("{feet} MSL"),
        AdvisoryAltitudeReference::AboveGroundLevel => format!("{feet} AGL"),
        AdvisoryAltitudeReference::FlightLevel => format!("FL{:03}", feet / 100),
        AdvisoryAltitudeReference::Surface => "SFC".to_owned(),
        _ => format!("{feet} ft"),
    }
}

const fn type_name(advisory_type: WeatherAdvisoryType) -> &'static str {
    match advisory_type {
        WeatherAdvisoryType::Sigmet => "SIGMET",
        WeatherAdvisoryType::ConvectiveSigmet => "CONV SIGMET",
        WeatherAdvisoryType::Airmet => "AIRMET",
        WeatherAdvisoryType::GAirmet => "G-AIRMET",
        WeatherAdvisoryType::CenterWeatherAdvisory => "CWA",
        _ => "ADVISORY",
    }
}
