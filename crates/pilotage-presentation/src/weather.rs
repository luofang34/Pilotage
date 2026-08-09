//! Weather display policy with a stopgap input decoder.
//!
//! The durable boundary uses typed Airmass weather values. This module
//! currently decodes interim product payloads and derives flight categories.
//! The category-to-style mapping is the durable display policy.

use airmass_core::{WeatherProductSnapshot, WeatherSnapshotEnvelope};
use serde::Deserialize;

use crate::policy::{
    ADVISORY_AIRMET_STYLE, ADVISORY_CONVECTIVE_STYLE, ADVISORY_CWA_STYLE, ADVISORY_G_AIRMET_STYLE,
    ADVISORY_SIGMET_STYLE, WEATHER_IFR_STYLE, WEATHER_LIFR_STYLE, WEATHER_MVFR_STYLE,
    WEATHER_UNKNOWN_STYLE, WEATHER_VFR_STYLE,
};
use crate::{
    Coordinate, CoordinateRing, DisplayBatch, PointFeature, PresentationAdapter, PresentationError,
    ShapeFeature,
};

/// Media type for a station weather observation payload.
pub const WEATHER_OBSERVATION_MEDIA_TYPE: &str =
    "application/vnd.pilotage.weather-observation+json;version=1";
/// Media type for an advisory polygon payload.
pub const WEATHER_ADVISORY_MEDIA_TYPE: &str =
    "application/vnd.pilotage.weather-advisory+json;version=1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WeatherObservation {
    station_id: String,
    latitude_deg: f64,
    longitude_deg: f64,
    ceiling_ft_agl: Option<u32>,
    visibility_statute_miles: Option<f64>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AdvisoryKind {
    Sigmet,
    ConvectiveSigmet,
    Airmet,
    GAirmet,
    CenterWeatherAdvisory,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdvisoryShape {
    kind: AdvisoryKind,
    label: Option<String>,
    rings: Vec<Vec<WeatherCoordinate>>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WeatherCoordinate {
    latitude_deg: f64,
    longitude_deg: f64,
}

pub(crate) fn features_for_weather(
    envelope: &WeatherSnapshotEnvelope,
) -> Result<DisplayBatch, PresentationError> {
    let mut batch = PresentationAdapter::new().empty_batch();
    for product in envelope.snapshot().products() {
        let Some(available) = product.state().available() else {
            batch.omitted_products = batch.omitted_products.wrapping_add(1);
            continue;
        };
        let payload = &available.payload().value;
        match payload.media_type.as_str() {
            WEATHER_OBSERVATION_MEDIA_TYPE => {
                batch
                    .points
                    .push(observation_feature(envelope, product, &payload.bytes)?)
            }
            WEATHER_ADVISORY_MEDIA_TYPE => {
                batch
                    .shapes
                    .push(advisory_feature(envelope, product, &payload.bytes)?)
            }
            _ => batch.omitted_products = batch.omitted_products.wrapping_add(1),
        }
    }
    Ok(batch)
}

fn observation_feature(
    envelope: &WeatherSnapshotEnvelope,
    product: &WeatherProductSnapshot,
    bytes: &[u8],
) -> Result<PointFeature, PresentationError> {
    let value: WeatherObservation = decode_payload(product, WEATHER_OBSERVATION_MEDIA_TYPE, bytes)?;
    let coordinate = checked_coordinate(product, value.latitude_deg, value.longitude_deg)?;
    let label = observation_label(&value);
    Ok(PointFeature {
        id: weather_id(envelope, product),
        coordinate,
        style_id: flight_category_style(&value).into(),
        label: Some(label),
        rotation_deg: 0.0,
        producer_instance_id: envelope.producer_instance_id().get(),
        snapshot_revision: envelope.snapshot_revision().get(),
    })
}

fn advisory_feature(
    envelope: &WeatherSnapshotEnvelope,
    product: &WeatherProductSnapshot,
    bytes: &[u8],
) -> Result<ShapeFeature, PresentationError> {
    let value: AdvisoryShape = decode_payload(product, WEATHER_ADVISORY_MEDIA_TYPE, bytes)?;
    let rings = advisory_rings(product, &value.rings)?;
    Ok(ShapeFeature {
        id: weather_id(envelope, product),
        rings,
        style_id: advisory_style(value.kind).into(),
        label: value.label,
        producer_instance_id: envelope.producer_instance_id().get(),
        snapshot_revision: envelope.snapshot_revision().get(),
    })
}

fn decode_payload<T: for<'de> Deserialize<'de>>(
    product: &WeatherProductSnapshot,
    media_type: &str,
    bytes: &[u8],
) -> Result<T, PresentationError> {
    serde_json::from_slice(bytes).map_err(|source| PresentationError::WeatherPayloadJson {
        product_id: product.product_id().as_str().into(),
        media_type: media_type.into(),
        source,
    })
}

fn checked_coordinate(
    product: &WeatherProductSnapshot,
    latitude_deg: f64,
    longitude_deg: f64,
) -> Result<Coordinate, PresentationError> {
    Coordinate::checked(latitude_deg, longitude_deg).ok_or_else(|| {
        PresentationError::InvalidCoordinate {
            product_id: product.product_id().as_str().into(),
            latitude_deg,
            longitude_deg,
        }
    })
}

fn advisory_rings(
    product: &WeatherProductSnapshot,
    source: &[Vec<WeatherCoordinate>],
) -> Result<Vec<CoordinateRing>, PresentationError> {
    let mut rings = Vec::with_capacity(source.len());
    for ring in source {
        let mut coordinates = Vec::with_capacity(ring.len());
        for coordinate in ring {
            coordinates.push(checked_coordinate(
                product,
                coordinate.latitude_deg,
                coordinate.longitude_deg,
            )?);
        }
        if !is_closed_ring(&coordinates) {
            return Err(PresentationError::InvalidAdvisoryShape {
                product_id: product.product_id().as_str().into(),
            });
        }
        rings.push(CoordinateRing { coordinates });
    }
    if rings.is_empty() {
        return Err(PresentationError::InvalidAdvisoryShape {
            product_id: product.product_id().as_str().into(),
        });
    }
    Ok(rings)
}

fn is_closed_ring(ring: &[Coordinate]) -> bool {
    ring.len() >= 4 && ring.first() == ring.last()
}

fn flight_category_style(value: &WeatherObservation) -> &'static str {
    let ceiling = value.ceiling_ft_agl;
    let visibility = value.visibility_statute_miles;
    if ceiling.is_some_and(|feet| feet < 500) || visibility.is_some_and(|miles| miles < 1.0) {
        WEATHER_LIFR_STYLE
    } else if ceiling.is_some_and(|feet| feet < 1_000)
        || visibility.is_some_and(|miles| miles < 3.0)
    {
        WEATHER_IFR_STYLE
    } else if ceiling.is_some_and(|feet| feet <= 3_000)
        || visibility.is_some_and(|miles| miles <= 5.0)
    {
        WEATHER_MVFR_STYLE
    } else if ceiling.is_some() || visibility.is_some() {
        WEATHER_VFR_STYLE
    } else {
        WEATHER_UNKNOWN_STYLE
    }
}

fn advisory_style(kind: AdvisoryKind) -> &'static str {
    match kind {
        AdvisoryKind::Sigmet => ADVISORY_SIGMET_STYLE,
        AdvisoryKind::ConvectiveSigmet => ADVISORY_CONVECTIVE_STYLE,
        AdvisoryKind::Airmet => ADVISORY_AIRMET_STYLE,
        AdvisoryKind::GAirmet => ADVISORY_G_AIRMET_STYLE,
        AdvisoryKind::CenterWeatherAdvisory => ADVISORY_CWA_STYLE,
    }
}

fn observation_detail(value: &WeatherObservation) -> Option<String> {
    match (value.ceiling_ft_agl, value.visibility_statute_miles) {
        (Some(ceiling), Some(visibility)) => Some(format!("{ceiling} ft / {visibility:.1} sm")),
        (Some(ceiling), None) => Some(format!("{ceiling} ft")),
        (None, Some(visibility)) => Some(format!("{visibility:.1} sm")),
        (None, None) => None,
    }
}

fn observation_label(value: &WeatherObservation) -> String {
    match observation_detail(value) {
        Some(detail) => format!("{}\n{detail}", value.station_id),
        None => value.station_id.clone(),
    }
}

fn weather_id(envelope: &WeatherSnapshotEnvelope, product: &WeatherProductSnapshot) -> String {
    format!(
        "weather-{}-{}",
        envelope.producer_instance_id().get(),
        product.product_id().as_str()
    )
}
