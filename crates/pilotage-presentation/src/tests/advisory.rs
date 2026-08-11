use airmass_core::{
    AbsenceEvidence, AbsenceReason, AbsentField, AdvisoryAltitude, AdvisoryAltitudeBand,
    AdvisoryAltitudeReference, AdvisoryGeometry, AdvisoryPolygon, AdvisoryPosition, AdvisoryRing,
    AdvisoryValidity, DeliveryPath, DurationMillis, EvaluationTime, FieldState, MonotonicTime,
    ProducerInstanceId, ProductProvenance, QualityEvidence, RawProductIdentity, SourceEpoch,
    SourceId, SourceProductRevision, SourceRef, SourceTime, StoreConfig, TimeEvidence, TimedField,
    UtcInterval, UtcTime, WeatherAdvisory, WeatherAdvisoryId, WeatherAdvisoryType, WeatherIngress,
    WeatherOrigin, WeatherPayload, WeatherProduct, WeatherProductId, WeatherStationId,
    WeatherStore, WeatherValue,
};
use airmass_geojson::{FeatureDelta, Wgs84Position, map_snapshot_transition};

use crate::{PresentationAdapter, WEATHER_ADVISORY_LAYER_ID};

#[test]
fn an_advisory_becomes_a_shape_with_its_type_and_band() {
    // The outline alone cannot answer "does this affect my cruise altitude", so the shape
    // must carry the band. A regression here shows as an advisory that reaches the client
    // and draws nothing.
    let mut adapter = PresentationAdapter::new();
    for delta in advisory_deltas(WeatherAdvisoryType::ConvectiveSigmet) {
        adapter.apply_weather_delta(&delta);
    }

    let batch = adapter.adapt();

    assert_eq!(batch.shapes.len(), 1, "one advisory must draw one shape");
    let shape = &batch.shapes[0];
    assert_eq!(shape.layer_id, WEATHER_ADVISORY_LAYER_ID);
    assert_eq!(shape.style_id, "advisory-convective");
    assert_eq!(
        shape.label.as_deref(),
        Some("CONV SIGMET 2000 MSL-24000 MSL\nREPORTED ALTITUDE")
    );
    assert!(shape.uses_reported_altitude_fallback);
    assert_eq!(shape.rings.len(), 1);
    assert_eq!(shape.rings[0].coordinates.len(), 5);
    let style_ids: Vec<&str> = batch
        .shape_styles
        .iter()
        .map(|style| style.id.as_str())
        .collect();
    assert!(
        style_ids.contains(&shape.style_id.as_str()),
        "a shape must name a style the catalog carries"
    );
}

#[test]
fn each_advisory_type_selects_its_own_color() {
    let cases = [
        (WeatherAdvisoryType::Sigmet, "advisory-sigmet"),
        (WeatherAdvisoryType::ConvectiveSigmet, "advisory-convective"),
        (WeatherAdvisoryType::Airmet, "advisory-airmet"),
        (WeatherAdvisoryType::GAirmet, "advisory-g-airmet"),
        (WeatherAdvisoryType::CenterWeatherAdvisory, "advisory-cwa"),
    ];

    for (advisory_type, expected_style) in cases {
        let mut adapter = PresentationAdapter::new();
        for delta in advisory_deltas(advisory_type) {
            adapter.apply_weather_delta(&delta);
        }
        let batch = adapter.adapt();
        assert_eq!(batch.shapes.len(), 1);
        assert_eq!(batch.shapes[0].style_id, expected_style);
    }
}

#[test]
fn a_band_states_each_limit_in_its_own_reference() {
    // A flight level printed as a mean-sea-level height, or a surface limit printed as
    // zero feet, states an altitude the advisory never gave.
    let band = AdvisoryAltitudeBand::new(
        AdvisoryAltitude::new(0, AdvisoryAltitudeReference::Surface),
        AdvisoryAltitude::new(24_000, AdvisoryAltitudeReference::FlightLevel),
    )
    .expect("fixture band is ordered");
    let mut store = weather_store();
    let current = store
        .accept(
            advisory_ingress_with_band(WeatherAdvisoryType::Airmet, band),
            time(100),
        )
        .expect("an advisory must be valid")
        .into_publication()
        .expect("an advisory must publish");
    let mut adapter = PresentationAdapter::new();
    for delta in map_snapshot_transition(None, current.envelope(), &station_position) {
        adapter.apply_weather_delta(&delta);
    }

    let batch = adapter.adapt();

    assert_eq!(batch.shapes.len(), 1);
    assert_eq!(
        batch.shapes[0].label.as_deref(),
        Some("AIRMET SFC-FL240\nREPORTED ALTITUDE")
    );
}

#[test]
fn terrain_elevation_places_an_msl_advisory_above_the_surface() {
    let mut adapter = PresentationAdapter::new();
    for delta in advisory_deltas(WeatherAdvisoryType::Airmet) {
        adapter
            .apply_weather_delta_with_terrain_blocking(&delta, |_| {
                Ok::<_, std::convert::Infallible>(Some(400.0))
            })
            .expect("terrain reader is infallible");
    }

    let batch = adapter.adapt();
    let shape = &batch.shapes[0];
    let base = shape
        .base_above_terrain_m
        .expect("an advisory states its floor");

    assert!((base - (2_000.0 * 0.3048 - 400.0)).abs() < 1e-6);
    assert!(!shape.uses_reported_altitude_fallback);
    assert_eq!(shape.label.as_deref(), Some("AIRMET 2000 MSL-24000 MSL"));
}

#[test]
fn an_agl_advisory_does_not_read_or_subtract_terrain() {
    let band = AdvisoryAltitudeBand::new(
        AdvisoryAltitude::new(2_000, AdvisoryAltitudeReference::AboveGroundLevel),
        AdvisoryAltitude::new(3_000, AdvisoryAltitudeReference::AboveGroundLevel),
    )
    .expect("fixture band is ordered");
    let mut adapter = PresentationAdapter::new();
    let terrain_reads = Cell::new(0u32);
    for delta in advisory_deltas_with_band(WeatherAdvisoryType::Airmet, band) {
        adapter
            .apply_weather_delta_with_terrain_blocking(&delta, |_| {
                terrain_reads.set(terrain_reads.get().wrapping_add(1));
                Ok::<_, std::convert::Infallible>(Some(400.0))
            })
            .expect("terrain reader is infallible");
    }

    let batch = adapter.adapt();
    let shape = &batch.shapes[0];
    let base = shape
        .base_above_terrain_m
        .expect("an advisory states its floor");

    assert!((base - 2_000.0 * 0.3048).abs() < 1e-6);
    assert_eq!(terrain_reads.get(), 0);
    assert!(!shape.uses_reported_altitude_fallback);
    assert_eq!(shape.label.as_deref(), Some("AIRMET 2000 AGL-3000 AGL"));
}

#[test]
fn a_disabled_advisory_layer_withholds_the_shape_and_keeps_it() {
    let mut adapter = PresentationAdapter::new();
    assert!(adapter.set_layer_enabled(WEATHER_ADVISORY_LAYER_ID, false));
    for delta in advisory_deltas(WeatherAdvisoryType::Airmet) {
        adapter.apply_weather_delta(&delta);
    }

    assert!(adapter.adapt().shapes.is_empty());

    assert!(adapter.set_layer_enabled(WEATHER_ADVISORY_LAYER_ID, true));
    assert_eq!(adapter.adapt().shapes.len(), 1);
}

#[test]
fn clearing_weather_removes_the_advisory_shape() {
    // An advisory that outlives its reception would keep drawing a hazard that no longer
    // has a source behind it.
    let mut adapter = PresentationAdapter::new();
    for delta in advisory_deltas(WeatherAdvisoryType::Sigmet) {
        adapter.apply_weather_delta(&delta);
    }
    assert_eq!(adapter.adapt().shapes.len(), 1);

    adapter.clear_weather();

    assert!(adapter.adapt().shapes.is_empty());
}

#[test]
fn the_advisory_layer_reports_a_source_once_a_shape_arrives() {
    let mut adapter = PresentationAdapter::new();
    let absent = advisory_layer_state(&adapter);
    for delta in advisory_deltas(WeatherAdvisoryType::Sigmet) {
        adapter.apply_weather_delta(&delta);
    }

    assert_eq!(absent, "Absent");
    assert_eq!(advisory_layer_state(&adapter), "Stale");
}

fn advisory_layer_state(adapter: &PresentationAdapter) -> String {
    adapter
        .adapt()
        .layers
        .into_iter()
        .find(|control| control.id == WEATHER_ADVISORY_LAYER_ID)
        .expect("the advisory layer must have a control")
        .source_state_label
}

fn advisory_deltas(advisory_type: WeatherAdvisoryType) -> Vec<FeatureDelta> {
    advisory_deltas_with_band(advisory_type, band())
}

fn advisory_deltas_with_band(
    advisory_type: WeatherAdvisoryType,
    band: AdvisoryAltitudeBand,
) -> Vec<FeatureDelta> {
    let mut store = weather_store();
    let current = store
        .accept(advisory_ingress_with_band(advisory_type, band), time(100))
        .expect("an advisory must be valid")
        .into_publication()
        .expect("an advisory must publish");
    map_snapshot_transition(None, current.envelope(), &station_position)
}

fn weather_store() -> WeatherStore {
    WeatherStore::new(StoreConfig::default(), ProducerInstanceId::new(23))
        .expect("default weather store must be valid")
}

fn advisory_ingress_with_band(
    advisory_type: WeatherAdvisoryType,
    band: AdvisoryAltitudeBand,
) -> WeatherIngress {
    // The store rejects an advisory whose validity does not match the product period, so
    // the fixture states one interval in both places.
    let validity = AdvisoryValidity::Period(UtcInterval::new(
        UtcTime::from_unix_millis(0),
        UtcTime::from_unix_millis(600_000),
    ));
    let advisory = WeatherAdvisory::new(
        present(WeatherAdvisoryId::new("fixture-advisory")),
        present(advisory_type),
        present(validity),
        present(geometry()),
        absent(),
        present(band),
    );
    let product = WeatherProduct::new(
        field(WeatherPayload::new("application/octet-stream", Vec::new())),
        present(WeatherValue::Advisories(vec![advisory])),
        absent(),
        absent(),
        present(
            validity
                .lifecycle_interval()
                .expect("fixture validity is ordered"),
        ),
        present(DurationMillis::new(600_000)),
    );
    WeatherIngress::product(
        WeatherProductId::new("typed-advisories"),
        SourceProductRevision::new(1),
        product,
    )
}

fn geometry() -> AdvisoryGeometry {
    let corners = [
        (42_000_000, -72_000_000),
        (42_000_000, -70_000_000),
        (40_000_000, -70_000_000),
        (40_000_000, -72_000_000),
        (42_000_000, -72_000_000),
    ];
    let positions = corners
        .into_iter()
        .map(|(latitude_e6, longitude_e6)| {
            AdvisoryPosition::new(latitude_e6, longitude_e6).expect("fixture position is in range")
        })
        .collect();
    let ring = AdvisoryRing::new(positions).expect("fixture ring is closed");
    AdvisoryGeometry::Polygon(AdvisoryPolygon::new(vec![ring]).expect("fixture polygon has a ring"))
}

fn band() -> AdvisoryAltitudeBand {
    AdvisoryAltitudeBand::new(
        AdvisoryAltitude::new(2_000, AdvisoryAltitudeReference::MeanSeaLevel),
        AdvisoryAltitude::new(24_000, AdvisoryAltitudeReference::MeanSeaLevel),
    )
    .expect("fixture band is ordered")
}

fn field<T>(value: T) -> TimedField<T> {
    TimedField::new(
        value,
        TimeEvidence::new(MonotonicTime::from_micros(100), SourceTime::Unknown),
        QualityEvidence::Unavailable,
        provenance(),
    )
}

fn present<T>(value: T) -> FieldState<T> {
    FieldState::present(field(value))
}

fn absent<T>() -> FieldState<T> {
    FieldState::absent(AbsentField::new(
        AbsenceReason::NotObserved,
        AbsenceEvidence::Unknown,
    ))
}

fn provenance() -> ProductProvenance {
    ProductProvenance::new(
        SourceRef::new(SourceId::new("fixture"), SourceEpoch::new(1)),
        WeatherOrigin::Replay,
        DeliveryPath::new("typed-fixture"),
        Some(RawProductIdentity::new("fixture", "advisory")),
    )
}

fn station_position(_station_id: &WeatherStationId) -> Option<Wgs84Position> {
    None
}

const fn time(monotonic_micros: u64) -> EvaluationTime {
    EvaluationTime::new(
        UtcTime::from_unix_millis(0),
        MonotonicTime::from_micros(monotonic_micros),
    )
}
use std::cell::Cell;
