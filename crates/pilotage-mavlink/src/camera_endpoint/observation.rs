use crate::CameraReport;
use pilotage_protocol::wire;
use std::time::Instant;

pub(super) fn camera_observation(
    key: (u8, u8),
    report: &CameraReport,
    incarnation: pilotage_adapter_api::SourceIncarnation,
    started: Instant,
) -> Option<wire::PayloadZoomObservation> {
    let received = report.settings_received_at?;
    if received.elapsed() > crate::sampling::WITHHOLD_AFTER {
        return None;
    }
    let time = report.settings_time_boot_ms?;
    let value = turret_core::ZoomAbsolute::new(
        turret_core::ZoomUnit::TravelFraction,
        report.travel_fraction?,
    )
    .ok()?;
    let received_at = received
        .checked_duration_since(started)
        .and_then(|duration| u64::try_from(duration.as_micros()).ok())
        .map(turret_core::Tick)?;
    let observation = turret_core::Observation {
        value,
        source: turret_core::ObservationSource::DeviceMeasured,
        frame: None,
        sampled_at: None,
        received_at,
        epoch: report.epoch,
    };
    Some(pilotage_adapter_api::payload_zoom_observation_to_wire(
        &observation,
        wire::MeasurementStamp {
            source_id: u64::from(key.0) * 256 + u64::from(key.1),
            source_incarnation: incarnation.into_bytes().to_vec(),
            source_epoch: u32::try_from(report.epoch).unwrap_or(u32::MAX),
            sequence: time,
            acquired_at_ns: u64::from(time) * 1_000_000,
            clock: wire::MeasurementClock::VehicleBoot as i32,
            role: wire::SourceRole::PayloadDevice as i32,
            integrity: wire::SourceIntegrity::ChecksummedOnly as i32,
        },
    ))
}
