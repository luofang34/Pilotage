use airmass_core::{
    AbsenceEvidence, AbsenceReason, AbsentField, DeliveryPath, DurationMillis, EvaluationTime,
    FieldState, FlightCategory, MonotonicTime, ProducerInstanceId, ProductProvenance,
    QualityEvidence, RawProductIdentity, SourceEpoch, SourceId, SourceProductRevision, SourceRef,
    SourceTime, StoreConfig, TextWeatherReport, TimeEvidence, TimedField, UtcTime, WeatherIngress,
    WeatherOrigin, WeatherPayload, WeatherProduct, WeatherProductId, WeatherReportType,
    WeatherStationId, WeatherStore, WeatherValue,
};
use airmass_geojson::{FeatureDelta, Wgs84Position, map_snapshot_transition};

use crate::{PointChange, PresentationAdapter};

#[test]
fn typed_flight_categories_select_the_color_policy() {
    let cases = [
        (FlightCategory::Vfr, "weather-vfr"),
        (FlightCategory::Mvfr, "weather-mvfr"),
        (FlightCategory::Ifr, "weather-ifr"),
        (FlightCategory::Lifr, "weather-lifr"),
    ];

    for (category, expected_style) in cases {
        let delta = initial_delta(Some(category));
        let mut adapter = PresentationAdapter::new();
        let PointChange::Upsert { point } = adapter
            .apply_weather_delta(&delta)
            .expect("typed report must produce a point")
        else {
            panic!("typed report must produce an upsert");
        };

        assert_eq!(point.style_id, expected_style);
        assert_eq!(point.label.as_deref(), Some("KBOS"));
    }
}

#[test]
fn absent_flight_category_uses_the_unknown_color() {
    let delta = initial_delta(None);
    let mut adapter = PresentationAdapter::new();
    let PointChange::Upsert { point } = adapter
        .apply_weather_delta(&delta)
        .expect("typed report must produce a point")
    else {
        panic!("typed report must produce an upsert");
    };

    assert_eq!(point.style_id, "weather-unknown");
}

#[test]
fn expiry_removes_the_typed_weather_point() {
    let mut store = weather_store();
    let accepted = store
        .accept(
            report_ingress("KBOS", Some(FlightCategory::Ifr), 1, 1),
            time(100),
        )
        .expect("typed report must be valid")
        .into_publication()
        .expect("typed report must publish");
    let expired = store
        .advance_time(time(1_100))
        .expect("weather time must advance")
        .into_publication()
        .expect("expiry must publish");
    let mut adapter = PresentationAdapter::new();
    apply_all(
        &mut adapter,
        map_snapshot_transition(None, accepted.envelope(), &station_position),
    );

    let changes = map_snapshot_transition(
        Some(accepted.envelope()),
        expired.envelope(),
        &station_position,
    );
    assert!(matches!(changes.as_slice(), [FeatureDelta::Remove { .. }]));
    let change = adapter
        .apply_weather_delta(&changes[0])
        .expect("expiry must remove the point");

    assert!(matches!(change, PointChange::Remove { .. }));
    assert!(adapter.adapt().points.is_empty());
}

#[test]
fn replacement_removes_the_prior_report() {
    let mut store = weather_store();
    let first = store
        .accept(
            report_ingress("KBOS", Some(FlightCategory::Ifr), 1, 60_000),
            time(100),
        )
        .expect("first report must be valid")
        .into_publication()
        .expect("first report must publish");
    let second = store
        .accept(
            report_ingress("KJFK", Some(FlightCategory::Vfr), 2, 60_000),
            time(200),
        )
        .expect("replacement report must be valid")
        .into_publication()
        .expect("replacement report must publish");
    let mut adapter = PresentationAdapter::new();
    apply_all(
        &mut adapter,
        map_snapshot_transition(None, first.envelope(), &station_position),
    );
    apply_all(
        &mut adapter,
        map_snapshot_transition(Some(first.envelope()), second.envelope(), &station_position),
    );

    let batch = adapter.adapt();
    assert_eq!(batch.points.len(), 1);
    assert_eq!(batch.points[0].label.as_deref(), Some("KJFK"));
}

fn initial_delta(category: Option<FlightCategory>) -> FeatureDelta {
    let mut store = weather_store();
    let current = store
        .accept(report_ingress("KBOS", category, 1, 60_000), time(100))
        .expect("typed report must be valid")
        .into_publication()
        .expect("typed report must publish");
    map_snapshot_transition(None, current.envelope(), &station_position)
        .into_iter()
        .next()
        .expect("typed report must map to a feature")
}

fn apply_all(adapter: &mut PresentationAdapter, changes: Vec<FeatureDelta>) {
    for change in changes {
        assert!(adapter.apply_weather_delta(&change).is_some());
    }
}

fn weather_store() -> WeatherStore {
    WeatherStore::new(StoreConfig::default(), ProducerInstanceId::new(17))
        .expect("default weather store must be valid")
}

fn report_ingress(
    station_id: &str,
    category: Option<FlightCategory>,
    revision: u64,
    maximum_age_ms: u64,
) -> WeatherIngress {
    let report = TextWeatherReport::new(
        present(WeatherReportType::Metar),
        present(WeatherStationId::new(station_id)),
        absent(),
        field(format!("{station_id} typed report")),
        category.map_or_else(absent, present),
    );
    let product = WeatherProduct::new(
        field(WeatherPayload::new("application/octet-stream", Vec::new())),
        present(WeatherValue::TextReports(vec![report])),
        absent(),
        absent(),
        absent(),
        present(DurationMillis::new(maximum_age_ms)),
    );
    WeatherIngress::product(
        WeatherProductId::new("typed-reports"),
        SourceProductRevision::new(revision),
        product,
    )
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
        Some(RawProductIdentity::new("fixture", "typed-report")),
    )
}

fn station_position(station_id: &WeatherStationId) -> Option<Wgs84Position> {
    match station_id.as_str() {
        "KBOS" => Wgs84Position::new(42.3656, -71.0096),
        "KJFK" => Wgs84Position::new(40.6413, -73.7781),
        _ => None,
    }
}

const fn time(monotonic_micros: u64) -> EvaluationTime {
    EvaluationTime::new(
        UtcTime::from_unix_millis(0),
        MonotonicTime::from_micros(monotonic_micros),
    )
}
