//! Portable manager contracts and independent partial-axis observations.
use super::{GIMBAL_SCOPE, Px4Adapter};
use pilotage_adapter_api::GimbalAttitudeSample;
use pilotage_protocol::wire;

const ENDPOINT: &str = "mavlink.gimbal.selected";

pub(super) fn gimbal_capabilities(
    value: &turret_core::PayloadCapabilities,
) -> wire::PayloadCapabilities {
    let mut capabilities =
        pilotage_adapter_api::payload_capabilities_to_wire(ENDPOINT, value, &[GIMBAL_SCOPE]);
    capabilities.connection_role = wire::PayloadConnectionRole::MavlinkManager as i32;
    capabilities
}

impl Px4Adapter {
    pub(super) fn observe_gimbal_manager(&mut self) {
        let report = self
            .estimate
            .as_ref()
            .and_then(|source| source.state.lock().ok()?.fc_reports.gimbal_manager);
        if let Some((report, received)) = report
            && let Some(gimbal) = self.gimbal.as_mut()
        {
            gimbal.observe_manager(report, received);
        }
    }

    pub(super) fn payload_state(
        &self,
        sample: Option<&GimbalAttitudeSample>,
    ) -> Option<wire::PayloadState> {
        let gimbal = self.gimbal.as_ref()?;
        let (incarnation, _) = self.gimbal_stamp.identity();
        let mut state = wire::PayloadState {
            endpoint_id: ENDPOINT.to_owned(),
            capability_revision: gimbal.capabilities().revision,
            connection_incarnation: incarnation.into_bytes().to_vec(),
            connection_epoch: 1,
            command: gimbal.snapshot_cached().map(|snapshot| {
                pilotage_adapter_api::payload_progress_to_wire(
                    &snapshot.progress,
                    Some(&snapshot.command),
                    snapshot.updated_at,
                )
            }),
            ..Default::default()
        };
        if let Some(sample) = sample
            && let Some(device) = self
                .estimate
                .as_ref()
                .and_then(|source| source.state.lock().ok()?.gimbal_device)
        {
            let angles = crate::uplink::direct::euler_of(sample.quat_wxyz);
            let stamp = stamp(sample.stamp);
            let received_at = device
                .received_at
                .checked_duration_since(gimbal.started_at())
                .and_then(|duration| u64::try_from(duration.as_nanos()).ok())
                .map(|nanos| wire::MonoTimestamp { nanos });
            let axis = |angle: f32, rate: f32| wire::PayloadAxisObservation {
                angle_rad: angle.is_finite().then_some(angle),
                rate_rad_s: rate.is_finite().then_some(rate),
                stamp: Some(stamp.clone()),
                frame: wire::PayloadReferenceFrame::Unspecified as i32,
                source: wire::PayloadObservationSource::Transformed as i32,
                sampled_at: None,
                received_at,
                connection_epoch: 1,
            };
            state.pitch = Some(axis(angles[1], sample.rates_rps[1]));
            state.roll = Some(axis(angles[0], sample.rates_rps[0]));
            state.yaw = Some(axis(angles[2], sample.rates_rps[2]));
        }
        (state.pitch.is_some() || state.command.is_some()).then_some(state)
    }
}

fn stamp(value: pilotage_adapter_api::MeasurementStamp) -> wire::MeasurementStamp {
    wire::MeasurementStamp {
        source_id: value.source_id,
        source_incarnation: value.source_incarnation.into_bytes().to_vec(),
        source_epoch: value.source_epoch,
        sequence: value.sequence,
        acquired_at_ns: value.acquired_at_ns,
        role: wire::SourceRole::PayloadDevice as i32,
        integrity: wire::SourceIntegrity::ChecksummedOnly as i32,
        clock: wire::MeasurementClock::VehicleBoot as i32,
    }
}
