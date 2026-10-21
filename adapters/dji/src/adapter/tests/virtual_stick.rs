mod fencing;
mod progress;

use super::flight::{feed, feed_battery, feed_fc};
use super::harness::{Harness, packets, seal_duml};
use crate::adapter::{
    DjiVirtualStickDiagnosticError as Error, DjiVirtualStickDiagnosticPhase as Phase,
    DjiVirtualStickDiagnosticReason as Reason, DjiVirtualStickDiagnosticStatus,
};
use crate::{DumlFrame, flight::VirtualStickMode, logiclink::LogicLinkParser};
use pilotage_adapter_api::VehicleAdapter;
use std::time::Duration;

pub(super) fn ready() -> Harness {
    let mut h = super::flight::ready();
    h.adapter.experimental_virtual_stick_diagnostic = true;
    report(&mut h, 10, false, true, false);
    h.step(11);
    h
}

pub(super) fn report(h: &mut Harness, sequence: u16, enabled: bool, multiple: bool, novice: bool) {
    let [lo, hi] = sequence.to_le_bytes();
    let mut bytes = vec![0x55, 91, 4, 0, 14, 2, lo, hi, 0, 3, 0x44];
    let mut payload = [0; 78];
    let config = (u16::from(multiple) << 6) | (u16::from(novice) << 11);
    payload[20..22].copy_from_slice(&config.to_le_bytes());
    payload[32] = u8::from(enabled) << 1;
    bytes.extend(payload);
    bytes.extend([0, 0]);
    seal_duml(&mut bytes);
    feed(h, &bytes);
}

pub(super) fn ack(h: &mut Harness, mode: VirtualStickMode, sequence: u16, code: u8) {
    let mut bytes = *mode.packet(sequence).bytes();
    bytes[4] = 3;
    bytes[5] = 2;
    bytes[8] = 0x80;
    bytes[11] = code;
    seal_duml(&mut bytes);
    feed(h, &bytes);
}

fn request(h: &mut Harness, now: u64, until: u64) -> Result<(), Error> {
    h.adapter
        .request_virtual_stick_at(41, Duration::from_millis(now), Duration::from_millis(until))
}

fn status(h: &Harness) -> DjiVirtualStickDiagnosticStatus {
    h.adapter.virtual_stick_diagnostic_status_cached().unwrap()
}

pub(super) fn commands(h: &Harness) -> Vec<DumlFrame> {
    let mut parser = LogicLinkParser::new();
    let mut result = Vec::new();
    parser
        .push(&h.written().unwrap(), |envelope| {
            result.extend(packets(envelope.payload))
        })
        .unwrap();
    parser.finish().unwrap();
    result
        .into_iter()
        .filter(|frame| frame.command_set() == 3)
        .collect()
}

#[test]
fn explicit_opt_in_never_adds_velocity_capability_or_starts_a_probe() {
    let mut h = ready();
    assert!(commands(&h).is_empty());
    let capabilities = h.adapter.capabilities();
    assert!(capabilities.vehicles[0].scopes.iter().all(|scope| {
        scope
            .intents
            .iter()
            .all(|intent| intent.family != pilotage_protocol::IntentFamily::Velocity)
    }));
    h.adapter.experimental_virtual_stick_diagnostic = false;
    assert_eq!(request(&mut h, 11, 100), Err(Error::Disabled));
    h.step(20);
    assert!(commands(&h).is_empty());
}

#[test]
fn complete_pair_keeps_ack_and_observation_independent_and_never_sends_velocity() {
    let mut h = ready();
    request(&mut h, 11, 100).unwrap();
    h.step(12);
    assert_eq!(commands(&h).len(), 1);
    ack(&mut h, VirtualStickMode::Enable, 2, 0);
    h.step(13);
    assert_eq!(status(&h).phase, Phase::Enable);
    assert_eq!(status(&h).enable.observed_at, None);
    report(&mut h, 11, true, true, false);
    h.step(14);
    assert_eq!(commands(&h).len(), 2);
    assert_eq!(status(&h).phase, Phase::Disable);
    assert_eq!(
        status(&h).disable.written_at,
        Some(Duration::from_millis(14))
    );
    ack(&mut h, VirtualStickMode::Disable, 3, 0);
    h.step(15);
    assert_eq!(status(&h).phase, Phase::Disable);
    report(&mut h, 12, false, true, false);
    h.step(16);
    assert_eq!(status(&h).phase, Phase::Finished);
    assert_eq!(status(&h).reason, None);
    assert!(status(&h).disable.observed_at.is_some());
    assert_eq!(
        commands(&h)
            .iter()
            .map(|frame| (frame.command_id(), frame.payload()[0]))
            .collect::<Vec<_>>(),
        [(0x80, 1), (0x80, 2)]
    );
    assert_eq!(request(&mut h, 20, 100), Err(Error::AlreadyAttempted));
}
