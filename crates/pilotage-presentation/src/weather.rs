//! Weather display policy for typed Airmass feature changes.

use airmass_core::{FlightCategory, WeatherReportType};
use airmass_geojson::{FeatureDelta, TextReportFeature, WeatherFeatureId};

use crate::layer::WEATHER_REPORT_LAYER_ID;
use crate::policy::{
    WEATHER_IFR_STYLE, WEATHER_LIFR_STYLE, WEATHER_MVFR_STYLE, WEATHER_UNKNOWN_STYLE,
    WEATHER_VFR_STYLE,
};
use crate::{Coordinate, PointChange, PointFeature};

pub(crate) fn feature_id_for_delta(delta: &FeatureDelta) -> Option<String> {
    match delta {
        FeatureDelta::Upsert(feature) => Some(feature_id(feature.id())),
        FeatureDelta::Remove { id, .. } => Some(feature_id(id)),
        _ => None,
    }
}

pub(crate) fn point_change(
    delta: &FeatureDelta,
    current: Option<&PointFeature>,
) -> Option<PointChange> {
    match delta {
        FeatureDelta::Upsert(feature) => {
            point_for_report(feature.text_report()?).map(|point| PointChange::Upsert { point })
        }
        FeatureDelta::Remove { id, .. } => {
            let point = current?;
            Some(PointChange::Remove {
                id: feature_id(id),
                transfer_to: None,
                producer_instance_id: point.producer_instance_id,
                snapshot_revision: point.snapshot_revision,
            })
        }
        _ => None,
    }
}

fn point_for_report(feature: &TextReportFeature) -> Option<PointFeature> {
    let position = feature.position();
    let coordinate = Coordinate::checked(position.latitude_deg(), position.longitude_deg())?;
    let revisions = feature.revisions();
    Some(PointFeature {
        id: feature_id(feature.id()),
        layer_id: WEATHER_REPORT_LAYER_ID.into(),
        coordinate,
        style_id: category_style(
            feature
                .report()
                .flight_category()
                .as_present()
                .map(|field| field.value),
        )
        .into(),
        label: Some(feature.id().station_id().as_str().into()),
        altitude_ft: None,
        rotation_deg: 0.0,
        producer_instance_id: feature.id().producer_instance_id().get(),
        snapshot_revision: revisions.snapshot_revision().get(),
    })
}

fn category_style(category: Option<FlightCategory>) -> &'static str {
    match category {
        Some(FlightCategory::Vfr) => WEATHER_VFR_STYLE,
        Some(FlightCategory::Mvfr) => WEATHER_MVFR_STYLE,
        Some(FlightCategory::Ifr) => WEATHER_IFR_STYLE,
        Some(FlightCategory::Lifr) => WEATHER_LIFR_STYLE,
        _ => WEATHER_UNKNOWN_STYLE,
    }
}

fn feature_id(id: &WeatherFeatureId) -> String {
    let product_id = id.product_id().as_str();
    let station_id = id.station_id().as_str();
    format!(
        "airmass:{}:{}:{}:{}:{}:{}:{}:point",
        id.producer_instance_id().get(),
        product_id.len(),
        product_id,
        report_type_name(id.report_type()),
        station_id.len(),
        station_id,
        id.occurrence(),
    )
}

const fn report_type_name(report_type: WeatherReportType) -> &'static str {
    match report_type {
        WeatherReportType::Metar => "metar",
        WeatherReportType::Speci => "speci",
        WeatherReportType::Taf => "taf",
        WeatherReportType::AmendedTaf => "amended_taf",
        WeatherReportType::Pirep => "pirep",
        WeatherReportType::WindsAndTemperaturesAloft => "winds_and_temperatures_aloft",
        _ => "text_report",
    }
}
