use airmass_core::{
    AbsenceEvidence, AbsenceReason, AbsentField, DeliveryPath, EvaluationTime, FieldState,
    MonotonicTime, ProducerInstanceId, ProductProvenance, QualityEvidence, RawProductIdentity,
    SourceEpoch, SourceId, SourceProductRevision, SourceRef, SourceTime, StoreConfig, TimeEvidence,
    TimedField, UtcTime, WeatherOrigin, WeatherPayload, WeatherProduct, WeatherProductId,
    WeatherSnapshotEnvelope, WeatherStore,
};

use crate::{
    PresentationAdapter, PresentationError, WEATHER_ADVISORY_MEDIA_TYPE,
    WEATHER_OBSERVATION_MEDIA_TYPE,
};

#[test]
fn observation_thresholds_select_the_flight_category_style() {
    let payload = br#"{
        "station_id":"KBOS",
        "latitude_deg":42.3656,
        "longitude_deg":-71.0096,
        "ceiling_ft_agl":800,
        "visibility_statute_miles":6.0
    }"#;
    let envelope = weather_envelope(WEATHER_OBSERVATION_MEDIA_TYPE, payload);
    let batch = PresentationAdapter::new()
        .adapt(Some(&envelope))
        .expect("weather conversion must succeed");
    let point = batch.points.first().expect("station point must exist");

    assert_eq!(point.style_id, "weather-ifr");
    assert_eq!(point.label.as_deref(), Some("KBOS\n800 ft / 6.0 sm"));
}

#[test]
fn three_thousand_foot_ceiling_is_marginal_vfr() {
    let payload = br#"{
        "station_id":"KBOS",
        "latitude_deg":42.3656,
        "longitude_deg":-71.0096,
        "ceiling_ft_agl":3000,
        "visibility_statute_miles":10.0
    }"#;
    let envelope = weather_envelope(WEATHER_OBSERVATION_MEDIA_TYPE, payload);
    let batch = PresentationAdapter::new()
        .adapt(Some(&envelope))
        .expect("weather conversion must succeed");
    let point = batch.points.first().expect("station point must exist");

    assert_eq!(point.style_id, "weather-mvfr");
}

#[test]
fn advisory_payload_becomes_a_closed_polygon() {
    let payload = br#"{
        "kind":"convective_sigmet",
        "label":"SIGC-1",
        "rings":[[
            {"latitude_deg":40.0,"longitude_deg":-75.0},
            {"latitude_deg":41.0,"longitude_deg":-75.0},
            {"latitude_deg":41.0,"longitude_deg":-74.0},
            {"latitude_deg":40.0,"longitude_deg":-75.0}
        ]]
    }"#;
    let envelope = weather_envelope(WEATHER_ADVISORY_MEDIA_TYPE, payload);
    let batch = PresentationAdapter::new()
        .adapt(Some(&envelope))
        .expect("advisory conversion must succeed");
    let shape = batch.shapes.first().expect("advisory shape must exist");

    assert_eq!(shape.style_id, "advisory-convective");
    assert_eq!(shape.rings[0].coordinates.len(), 4);
}

#[test]
fn unsupported_weather_payload_is_counted_and_not_guessed() {
    let envelope = weather_envelope("application/octet-stream", &[1, 2, 3]);
    let batch = PresentationAdapter::new()
        .adapt(Some(&envelope))
        .expect("unknown media type must not fail the batch");

    assert_eq!(batch.omitted_products, 1);
    assert!(batch.points.is_empty());
    assert!(batch.shapes.is_empty());
}

#[test]
fn open_advisory_ring_fails_with_product_context() {
    let payload = br#"{
        "kind":"airmet",
        "label":null,
        "rings":[[
            {"latitude_deg":40.0,"longitude_deg":-75.0},
            {"latitude_deg":41.0,"longitude_deg":-75.0},
            {"latitude_deg":41.0,"longitude_deg":-74.0},
            {"latitude_deg":40.0,"longitude_deg":-74.0}
        ]]
    }"#;
    let envelope = weather_envelope(WEATHER_ADVISORY_MEDIA_TYPE, payload);
    let error = PresentationAdapter::new()
        .adapt(Some(&envelope))
        .expect_err("open ring must fail");

    assert!(matches!(
        error,
        PresentationError::InvalidAdvisoryShape { product_id } if product_id == "fixture"
    ));
}

fn weather_envelope(media_type: &str, bytes: &[u8]) -> WeatherSnapshotEnvelope {
    let provenance = ProductProvenance::new(
        SourceRef::new(SourceId::new("fixture"), SourceEpoch::new(1)),
        WeatherOrigin::Simulation,
        DeliveryPath::new("unit-test"),
        Some(RawProductIdentity::new("fixture", "product")),
    );
    let payload = TimedField::new(
        WeatherPayload::new(media_type, bytes.to_vec()),
        TimeEvidence::new(MonotonicTime::from_millis(10), SourceTime::Unknown),
        QualityEvidence::Unavailable,
        provenance,
    );
    let product = WeatherProduct::new(payload, absent(), absent(), absent(), absent());
    let mut store = WeatherStore::new(StoreConfig::default(), ProducerInstanceId::new(11))
        .expect("store configuration must be valid");
    store
        .accept(
            airmass_core::WeatherIngress::product(
                WeatherProductId::new("fixture"),
                SourceProductRevision::new(1),
                product,
            ),
            EvaluationTime::new(
                UtcTime::from_unix_millis(10),
                MonotonicTime::from_millis(10),
            ),
        )
        .expect("fixture product must publish");
    store.capture().into_envelope()
}

fn absent<T>() -> FieldState<T> {
    FieldState::absent(AbsentField::new(
        AbsenceReason::NotObserved,
        AbsenceEvidence::Unknown,
    ))
}
