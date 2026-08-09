use crate::{
    LayerSourceState, PresentationAdapter, RadioBand, RadioReceiverObservation,
    RadioReceptionState, SourceObservation, TERRAIN_LAYER_ID, TRAFFIC_LAYER_ID,
    WEATHER_ADVISORY_LAYER_ID, WEATHER_REPORT_LAYER_ID,
};

#[test]
fn every_mvp_layer_starts_on_and_reports_absence() {
    let batch = PresentationAdapter::new().adapt();

    assert_eq!(batch.layers.len(), 4);
    for layer in &batch.layers {
        assert!(layer.enabled);
    }
    assert_eq!(
        source_state(&batch, TERRAIN_LAYER_ID),
        LayerSourceState::Absent
    );
    assert_eq!(
        source_state(&batch, TRAFFIC_LAYER_ID),
        LayerSourceState::Absent
    );
    assert_eq!(
        source_state(&batch, WEATHER_REPORT_LAYER_ID),
        LayerSourceState::Absent
    );
    assert_eq!(
        source_state(&batch, WEATHER_ADVISORY_LAYER_ID),
        LayerSourceState::Absent
    );
    assert!(
        layer(&batch, WEATHER_REPORT_LAYER_ID)
            .source_detail
            .contains("does not mean clear weather")
    );
}

#[test]
fn raw_source_facts_select_live_and_stale_states() {
    let mut adapter = PresentationAdapter::new();
    adapter.observe_sources(SourceObservation {
        terrain_available: true,
        weather_positions_available: true,
        radio_state: RadioReceptionState::Streaming,
        radio_receivers: vec![
            RadioReceiverObservation {
                band: RadioBand::Adsb1090,
                state: RadioReceptionState::Streaming,
            },
            RadioReceiverObservation {
                band: RadioBand::Uat978,
                state: RadioReceptionState::Ready,
            },
        ],
    });
    let batch = adapter.adapt();

    assert_eq!(
        source_state(&batch, TERRAIN_LAYER_ID),
        LayerSourceState::Live
    );
    assert_eq!(
        source_state(&batch, TRAFFIC_LAYER_ID),
        LayerSourceState::Live
    );
    assert_eq!(
        source_state(&batch, WEATHER_REPORT_LAYER_ID),
        LayerSourceState::Stale
    );
    assert_eq!(
        source_state(&batch, WEATHER_ADVISORY_LAYER_ID),
        LayerSourceState::Absent
    );
}

#[test]
fn live_1090_receiver_does_not_make_weather_live() {
    let mut adapter = PresentationAdapter::new();
    adapter.observe_sources(SourceObservation {
        terrain_available: true,
        weather_positions_available: true,
        radio_state: RadioReceptionState::Streaming,
        radio_receivers: vec![RadioReceiverObservation {
            band: RadioBand::Adsb1090,
            state: RadioReceptionState::Streaming,
        }],
    });

    assert_eq!(
        source_state(&adapter.adapt(), WEATHER_REPORT_LAYER_ID),
        LayerSourceState::Absent
    );
}

#[test]
fn unknown_layer_identity_does_not_change_the_catalog() {
    let mut adapter = PresentationAdapter::new();

    assert!(!adapter.set_layer_enabled("future-layer", false));
    assert_eq!(adapter.adapt().layers.len(), 4);
}

fn source_state(batch: &crate::DisplayBatch, id: &str) -> LayerSourceState {
    layer(batch, id).source_state
}

fn layer<'a>(batch: &'a crate::DisplayBatch, id: &str) -> &'a crate::LayerControl {
    batch
        .layers
        .iter()
        .find(|layer| layer.id == id)
        .expect("the layer catalog must contain the expected identity")
}
