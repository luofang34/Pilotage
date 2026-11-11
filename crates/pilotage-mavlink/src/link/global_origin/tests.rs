use std::sync::{Arc, Mutex};
use std::time::Duration;

use web_time::Instant;

use crate::codec::{FcMessage, GPS_GLOBAL_ORIGIN_ID, encode_frame_v2_from, parse_datagram};
use crate::link::{LinkState, apply_messages_at};

/// A GPS_GLOBAL_ORIGIN frame from system 1, component 1, with MAVLink 2's
/// trailing zero bytes trimmed as a real sender trims them.
fn origin_frame(lat_lon: [i32; 2], altitude_mm: i32) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&lat_lon[0].to_le_bytes());
    payload.extend_from_slice(&lat_lon[1].to_le_bytes());
    payload.extend_from_slice(&altitude_mm.to_le_bytes());
    payload.extend_from_slice(&0_u64.to_le_bytes());
    while payload.len() > 1 && payload.last() == Some(&0) {
        payload.pop();
    }
    let mut frame = vec![0_u8; 10 + payload.len() + 2];
    let written = encode_frame_v2_from(0, GPS_GLOBAL_ORIGIN_ID, &payload, 39, 1, 1, &mut frame);
    frame.truncate(written);
    frame
}

fn received(state: &Arc<Mutex<LinkState>>, frame: &[u8], now: Instant) {
    let mut messages = Vec::new();
    let stats = parse_datagram(frame, &mut messages);
    assert_eq!(stats.decoded, 1, "the frame decodes");
    apply_messages_at(state, &messages, 0, 0, now);
}

fn state() -> Arc<Mutex<LinkState>> {
    Arc::new(Mutex::new(LinkState::default()))
}

#[test]
fn a_trimmed_origin_frame_decodes_its_latitude_and_longitude_and_no_altitude() {
    // A longitude whose high bytes are zero ends the trimmed payload.
    let frame = origin_frame([395_000_000, 255], 0);
    assert!(frame.len() < 10 + 20 + 2, "the sender trimmed the payload");
    let mut messages = Vec::new();
    parse_datagram(&frame, &mut messages);
    assert_eq!(
        messages.first().map(|(_, message)| *message),
        Some(FcMessage::GlobalOrigin {
            lat_lon: [395_000_000, 255]
        })
    );
}

#[test]
fn the_origin_is_cached_and_a_moved_origin_counts_a_rebase() {
    let state = state();
    let start = Instant::now();
    received(
        &state,
        &origin_frame([395_000_000, -755_000_000], 12_000),
        start,
    );
    let first = state.lock().expect("lock").global_origin.expect("cached");
    assert_eq!(first.lat_lon, [395_000_000, -755_000_000]);
    assert_eq!(first.rebases, 0);

    let later = start + Duration::from_secs(1);
    received(
        &state,
        &origin_frame([395_000_000, -755_000_000], 12_000),
        later,
    );
    let repeated = state.lock().expect("lock").global_origin.expect("cached");
    assert_eq!(repeated.rebases, 0, "the same origin again is no rebase");
    assert_eq!(repeated.received_at, later);

    let moved = start + Duration::from_secs(2);
    received(
        &state,
        &origin_frame([395_100_000, -755_000_000], 12_000),
        moved,
    );
    let rebased = state.lock().expect("lock").global_origin.expect("cached");
    assert_eq!(rebased.lat_lon, [395_100_000, -755_000_000]);
    assert_eq!(rebased.rebases, 1);
}

#[test]
fn an_origin_at_zero_zero_is_unset_and_leaves_the_cache() {
    let state = state();
    let start = Instant::now();
    received(&state, &origin_frame([0, 0], 0), start);
    assert_eq!(state.lock().expect("lock").global_origin, None);
    received(&state, &origin_frame([395_000_000, 1], 0), start);
    received(&state, &origin_frame([0, 0], 0), start);
    let kept = state.lock().expect("lock").global_origin.expect("cached");
    assert_eq!(kept.lat_lon, [395_000_000, 1]);
}
