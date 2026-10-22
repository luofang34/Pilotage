mod admission;
mod fencing;
mod progress;
mod session;

use pilotage_adapter_api::{FlightProcedureSample, VehicleAdapter};
use pilotage_protocol::{ControlAction, ScopeId, ScopedControlFrame, SequenceNum};

use super::harness::{Harness, captured_hex_frames, frame, packets, seal_duml};
use crate::adapter::{DjiControlMode, FLIGHT_SCOPE, input::Framing};
use crate::{DumlFrame, logiclink::LogicLinkParser};

pub(super) fn ready() -> Harness {
    let mut h = Harness::flight(DjiControlMode::Observed, Framing::LogicLink);
    feed(&mut h, &probe());
    feed_fc(&mut h, 1, 0);
    feed_battery(&mut h, 1, 50);
    remote(&mut h, 1, [1024; 4]);
    h.step(10);
    h
}

pub(super) fn request(action: ControlAction, id: u32) -> ScopedControlFrame {
    let mut value = frame(0.0);
    value.scope = ScopeId::new(FLIGHT_SCOPE);
    value.sequence = SequenceNum::new(0);
    value.intent = None;
    value.actions = vec![action];
    value.action_ids = vec![id];
    value
}

pub(super) fn feed(h: &mut Harness, bytes: &[u8]) {
    h.feed(&crate::logiclink::encode_duml(bytes)).unwrap();
}

pub(super) fn remote(h: &mut Harness, sequence: u16, axes: [u16; 4]) {
    let [lo, hi] = sequence.to_le_bytes();
    let mut bytes = vec![0x55, 26, 4, 0, 6, 2, lo, hi, 0, 6, 5];
    for axis in axes {
        bytes.extend(axis.to_le_bytes());
    }
    bytes.extend([0, 4, 0, 0, 0, 0, 0]);
    seal_duml(&mut bytes);
    feed(h, &bytes);
}

pub(super) fn feed_fc(h: &mut Harness, sequence: u16, flags: u32) {
    let mut bytes = captured_hex_frames(include_str!(
        "../airframe/fixtures/mini3-reported-attitude.hex"
    ))
    .remove(0);
    bytes[6..8].copy_from_slice(&sequence.to_le_bytes());
    bytes[43..47].copy_from_slice(&flags.to_le_bytes());
    seal_duml(&mut bytes);
    feed(h, &bytes);
}

pub(super) fn feed_battery(h: &mut Harness, sequence: u16, percent: u8) {
    let mut bytes =
        captured_hex_frames(include_str!("../battery/fixtures/mini3-battery-status.hex")).remove(0);
    bytes[6..8].copy_from_slice(&sequence.to_le_bytes());
    bytes[37] = percent;
    seal_duml(&mut bytes);
    feed(h, &bytes);
}

fn probe() -> Vec<u8> {
    let mut bytes = vec![0x55, 13, 4, 0, 6, 2, 1, 0, 0x80, 0, 1, 0, 0];
    seal_duml(&mut bytes);
    bytes
}

pub(super) fn ack(sequence: u16, code: u8) -> Vec<u8> {
    let [lo, hi] = sequence.to_le_bytes();
    let mut bytes = vec![0x55, 14, 4, 0, 3, 2, lo, hi, 0x80, 3, 0x2a, code, 0, 0];
    seal_duml(&mut bytes);
    bytes
}

fn commands(h: &Harness) -> Vec<DumlFrame> {
    let mut mux = LogicLinkParser::new();
    let mut result = Vec::new();
    mux.push(&h.written().unwrap(), |envelope| {
        result.extend(packets(envelope.payload))
    })
    .unwrap();
    mux.finish().unwrap();
    result
        .into_iter()
        .filter(|frame| frame.command_set() == 3 && frame.command_id() == 0x2a)
        .collect()
}

fn progress(h: &mut Harness) -> FlightProcedureSample {
    h.adapter
        .sample_telemetry()
        .samples
        .into_iter()
        .find_map(|sample| sample.flight_procedure)
        .unwrap()
}
