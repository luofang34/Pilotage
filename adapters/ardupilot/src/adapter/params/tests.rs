#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use pilotage_mavlink::codec::{FcMessage, FrameSource, ParamValue, param_id};
use pilotage_mavlink::link::apply_messages_at;
use pilotage_mavlink::{LinkState, ParamRequest};

use super::DirectParams;

fn report(state: &std::sync::Arc<std::sync::Mutex<LinkState>>, name: &str, value: ParamValue) {
    let source = FrameSource {
        system_id: 1,
        component_id: 1,
        frame_sequence: 0,
    };
    let message = FcMessage::ParamValue {
        param_id: param_id(name),
        value,
    };
    apply_messages_at(state, &[(source, message)], 0, 0, Instant::now());
}

#[test]
fn direct_flight_waits_for_both_parameters_and_sets_only_bit_three() {
    let state = std::sync::Arc::new(std::sync::Mutex::new(LinkState::default()));
    let (lane, mut requests) = mpsc::channel(16);
    let mut params = DirectParams::new(lane, 1, 1);
    let now = Instant::now();

    params.maintain(&state.lock().expect("lock"), now);
    let reads: Vec<_> = std::iter::from_fn(|| requests.try_recv().ok()).collect();
    assert_eq!(reads.len(), 2, "both parameters are read");
    assert!(params.ready_hover().is_none());

    // GUID_OPTIONS = 1 (arming from the transmitter), as ArduPilot sends
    // an integer: the number in the float field, typed INT32.
    report(
        &state,
        "GUID_OPTIONS",
        ParamValue::Int32(1.0_f32.to_bits() as i32),
    );
    report(&state, "MOT_THST_HOVER", ParamValue::Real32(0.39));
    params.maintain(&state.lock().expect("lock"), now);
    let set = requests.try_recv().expect("a write");
    assert_eq!(
        set.request,
        ParamRequest::Set("GUID_OPTIONS", ParamValue::Int32(9.0_f32.to_bits() as i32)),
        "an INT32 write carrying the number 9 in the float field"
    );
    assert!(
        params.ready_hover().is_none(),
        "not until the FC confirms the bit"
    );

    report(
        &state,
        "GUID_OPTIONS",
        ParamValue::Int32(9.0_f32.to_bits() as i32),
    );
    params.maintain(&state.lock().expect("lock"), now + Duration::from_secs(1));
    assert_eq!(params.ready_hover(), Some(0.39));
    assert!(
        requests.try_recv().is_err(),
        "no second write, no early re-read"
    );
}

#[test]
fn an_implausible_hover_thrust_is_not_believed() {
    let state = std::sync::Arc::new(std::sync::Mutex::new(LinkState::default()));
    let (lane, _requests) = mpsc::channel(16);
    let mut params = DirectParams::new(lane, 1, 1);
    report(
        &state,
        "GUID_OPTIONS",
        ParamValue::Int32(8.0_f32.to_bits() as i32),
    );
    report(&state, "MOT_THST_HOVER", ParamValue::Real32(0.0));
    params.maintain(&state.lock().expect("lock"), Instant::now());
    assert!(params.ready_hover().is_none());
}
