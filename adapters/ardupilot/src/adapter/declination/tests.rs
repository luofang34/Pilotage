#![allow(clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_adapter_api::MagneticVariationSource;
use pilotage_mavlink::codec::{FcMessage, FrameSource, ParamValue, param_id};
use pilotage_mavlink::link::apply_messages_at;
use pilotage_mavlink::{LinkState, ParamRequest};
use tokio::sync::mpsc;

use super::DeclinationParams;

fn report(state: &Arc<Mutex<LinkState>>, name: &str, value: ParamValue, at: Instant) {
    let source = FrameSource {
        system_id: 1,
        component_id: 1,
        frame_sequence: 0,
    };
    let message = FcMessage::ParamValue {
        param_id: param_id(name),
        value,
    };
    apply_messages_at(state, &[(source, message)], 0, 0, at);
}

fn autodec(enabled: bool) -> ParamValue {
    // COMPASS_AUTODEC is an integer: ArduPilot sends the number in the
    // float field, typed INT32.
    ParamValue::Int32(f32::from(u8::from(enabled)).to_bits() as i32)
}

#[test]
fn both_parameters_are_read_and_refreshed() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let (lane, mut requests) = mpsc::channel(16);
    let mut params = DeclinationParams::new(lane, 1, 1);
    let now = Instant::now();
    params.maintain(&state.lock().expect("lock"), now);
    let reads: Vec<_> = std::iter::from_fn(|| requests.try_recv().ok())
        .map(|outbound| outbound.request)
        .collect();
    assert_eq!(
        reads,
        [
            ParamRequest::Read("COMPASS_DEC"),
            ParamRequest::Read("COMPASS_AUTODEC")
        ]
    );
    params.maintain(&state.lock().expect("lock"), now + Duration::from_secs(1));
    assert!(requests.try_recv().is_err(), "no early re-read");
    params.maintain(&state.lock().expect("lock"), now + Duration::from_secs(5));
    assert_eq!(std::iter::from_fn(|| requests.try_recv().ok()).count(), 2);
}

#[test]
fn automatic_declination_is_the_fc_model_and_a_set_one_is_configured() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let (lane, _requests) = mpsc::channel(16);
    let mut params = DeclinationParams::new(lane, 1, 1);
    report(
        &state,
        "COMPASS_DEC",
        ParamValue::Real32(-0.2113),
        Instant::now(),
    );
    assert!(
        params.sample(&state.lock().expect("lock")).is_none(),
        "the source is unknown until COMPASS_AUTODEC arrives"
    );

    report(&state, "COMPASS_AUTODEC", autodec(true), Instant::now());
    params.maintain(&state.lock().expect("lock"), Instant::now());
    let model = params.sample(&state.lock().expect("lock")).expect("sample");
    assert_eq!(model.source, MagneticVariationSource::FcModel);
    assert_eq!(model.declination_rad, -0.2113);
    assert_eq!(model.stamp.sequence, 0);

    report(&state, "COMPASS_AUTODEC", autodec(false), Instant::now());
    report(
        &state,
        "COMPASS_DEC",
        ParamValue::Real32(0.05),
        Instant::now(),
    );
    params.maintain(&state.lock().expect("lock"), Instant::now());
    let configured = params.sample(&state.lock().expect("lock")).expect("sample");
    assert_eq!(configured.source, MagneticVariationSource::FcConfigured);
    assert_eq!(configured.declination_rad, 0.05);
    assert_eq!(configured.stamp.sequence, 1, "a new report is a new sample");
}

#[test]
fn an_unrefreshed_declination_is_withheld() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let (lane, _requests) = mpsc::channel(16);
    let params = DeclinationParams::new(lane, 1, 1);
    let long_ago = Instant::now()
        .checked_sub(Duration::from_secs(20))
        .expect("monotonic clock");
    report(&state, "COMPASS_DEC", ParamValue::Real32(-0.2113), long_ago);
    report(&state, "COMPASS_AUTODEC", autodec(true), Instant::now());
    assert!(params.sample(&state.lock().expect("lock")).is_none());
}
