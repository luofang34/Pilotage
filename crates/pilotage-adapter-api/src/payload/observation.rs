use pilotage_protocol::wire;
use turret_core::{Angle, AngularRate, Observation, ObservationSource, ZoomAbsolute};

/// Project measured or estimated angle without inventing a sample timestamp.
#[must_use]
pub fn payload_angle_observation_to_wire(
    value: &Observation<Angle>,
    stamp: wire::MeasurementStamp,
) -> wire::PayloadAxisObservation {
    let mut wire = axis(value, stamp);
    wire.angle_rad = Some(value.value.as_radians());
    wire
}

/// Project an independently observed angular rate.
#[must_use]
pub fn payload_rate_observation_to_wire(
    value: &Observation<AngularRate>,
    stamp: wire::MeasurementStamp,
) -> wire::PayloadAxisObservation {
    let mut wire = axis(value, stamp);
    wire.rate_rad_s = Some(value.value.as_radians_per_second());
    wire
}

fn axis<T>(value: &Observation<T>, stamp: wire::MeasurementStamp) -> wire::PayloadAxisObservation {
    wire::PayloadAxisObservation {
        source: source(value.source),
        frame: value
            .frame
            .map_or(wire::PayloadReferenceFrame::Unknown as i32, super::frame),
        sampled_at: value.sampled_at.and_then(super::timestamp),
        received_at: super::timestamp(value.received_at),
        connection_epoch: value.epoch,
        stamp: Some(stamp),
        ..Default::default()
    }
}

/// Project zoom in its declared unit; no calibration conversion is implied.
#[must_use]
pub fn payload_zoom_observation_to_wire(
    value: &Observation<ZoomAbsolute>,
    stamp: wire::MeasurementStamp,
) -> wire::PayloadZoomObservation {
    wire::PayloadZoomObservation {
        unit: super::zoom_unit(value.value.unit()),
        value: f64::from(value.value.value()),
        source: source(value.source),
        sampled_at: value.sampled_at.and_then(super::timestamp),
        received_at: super::timestamp(value.received_at),
        connection_epoch: value.epoch,
        stamp: Some(stamp),
    }
}

fn source(value: ObservationSource) -> i32 {
    (match value {
        ObservationSource::DeviceMeasured => wire::PayloadObservationSource::Measured,
        ObservationSource::TransformedMeasurement => wire::PayloadObservationSource::Transformed,
        ObservationSource::Estimated => wire::PayloadObservationSource::Estimated,
        ObservationSource::OperatorAssumed => wire::PayloadObservationSource::OperatorAssumed,
    }) as i32
}
