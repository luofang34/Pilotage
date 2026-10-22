use pilotage_adapter_api::{SourceIncarnation, VehicleAdapter};

use super::{
    TestResult,
    harness::{Harness, captured_hex_frames, packets},
};
use crate::adapter::DjiTelemetrySource;

#[test]
fn a_replacement_attachment_advances_aircraft_reports_without_inheriting_commands() -> TestResult {
    let mut first = Harness::mobile();
    first.feed_model(87, 20)?;
    first.step(100);
    let before = first.telemetry(100).samples[0]
        .fc_state
        .unwrap()
        .motor_state
        .unwrap()
        .stamp;
    let source = first.adapter.telemetry_source();
    first.adapter.fence_transport();
    let mut second = Harness::mobile_with_incarnation(SourceIncarnation::new([43; 16]));
    second.adapter = second
        .adapter
        .with_host_clock(std::time::Instant::now(), Some(source));
    assert!(second.telemetry(200).samples.is_empty());
    assert!(second.adapter.take_control_authority_losses().is_empty());
    second.feed_model(87, 1)?;
    let battery = captured_hex_frames(include_str!("../battery/fixtures/mini3-battery-status.hex"));
    second.feed(&crate::logiclink::encode_control(&packets(&battery[0])[0]))?;
    second.step(200);
    let batch = second.telemetry(200);
    let sample = &batch.samples[0];
    let motor = sample.fc_state.unwrap().motor_state.unwrap().stamp;
    let battery = sample.battery.unwrap().stamp;
    assert_eq!(motor.source_incarnation, before.source_incarnation);
    assert_eq!(motor.source_epoch, before.source_epoch.wrapping_add(1));
    assert_eq!(motor.sequence, 1);
    assert_eq!(battery.source_incarnation, motor.source_incarnation);
    assert_eq!(battery.source_epoch, motor.source_epoch);
    assert_eq!(
        second.adapter.observations.empty_state().connection_epoch,
        0
    );
    assert_eq!(
        second
            .adapter
            .observations
            .empty_state()
            .connection_incarnation,
        vec![43; 16]
    );
    // Only discovery is emitted; the replacement does not replay a control frame.
    let commands = packets(&second.written()?);
    assert_eq!(commands.len(), 1);
    assert_eq!(
        (commands[0].command_set(), commands[0].command_id()),
        (0, 1)
    );
    Ok(())
}

#[test]
fn aircraft_report_epoch_wraps_without_changing_the_session_identity() {
    let source = DjiTelemetrySource {
        incarnation: SourceIncarnation::new([7; 16]),
        epoch: u32::MAX,
    }
    .next_attachment();
    assert_eq!(source.epoch, 0);
    assert_eq!(source.incarnation, SourceIncarnation::new([7; 16]));
}
