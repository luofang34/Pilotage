use pilotage_protocol::wire;
use turret_core::{Capability, Implementation, UnavailableReason};

/// Project a Turret endpoint without guessing unknown support or feedback.
#[must_use]
pub fn payload_capabilities_to_wire(
    endpoint_id: &str,
    capabilities: &turret_core::PayloadCapabilities,
    resource_scopes: &[&str],
) -> wire::PayloadCapabilities {
    let (stop, bound) = stop(capabilities.gimbal.stop);
    wire::PayloadCapabilities {
        endpoint_id: endpoint_id.to_owned(),
        revision: capabilities.revision,
        gimbal: Some(wire::PayloadGimbalCapabilities {
            pitch: Some(axis(&capabilities.gimbal.axes.pitch)),
            roll: Some(axis(&capabilities.gimbal.axes.roll)),
            yaw: Some(axis(&capabilities.gimbal.axes.yaw)),
            recenter: Some(recenter(&capabilities.gimbal.recenter)),
            stop_guarantee: stop,
            stop_bound_ms: bound,
        }),
        zoom: Some(zoom(&capabilities.zoom)),
        resource_scopes: resource_scopes
            .iter()
            .map(|scope| (*scope).to_owned())
            .collect(),
        connection_role: wire::PayloadConnectionRole::Unknown as i32,
        connection_incarnation: Vec::new(),
        connection_epoch: 0,
    }
}

fn declaration<T>(capability: &Capability<T>) -> (i32, Option<&T>, String) {
    use wire::PayloadSupportState as Support;
    match capability {
        Capability::Unknown => (Support::Unknown as i32, None, String::new()),
        Capability::Unsupported => (Support::Unsupported as i32, None, String::new()),
        Capability::Available(value) => (Support::Available as i32, Some(value), String::new()),
        Capability::Unavailable { capability, reason } => (
            Support::Unavailable as i32,
            Some(capability),
            reason_text(*reason).to_owned(),
        ),
    }
}

fn reason_text(reason: UnavailableReason) -> &'static str {
    match reason {
        UnavailableReason::Disconnected => "disconnected",
        UnavailableReason::Transport => "transport unavailable",
        UnavailableReason::DeviceMode => "device mode",
        UnavailableReason::MissingFeedback => "feedback unavailable",
        UnavailableReason::MissingCalibration => "calibration unavailable",
        UnavailableReason::NoAuthority => "authority unavailable",
        UnavailableReason::Policy => "disabled by policy",
    }
}

fn implementation(value: Implementation) -> i32 {
    (match value {
        Implementation::Native => wire::PayloadImplementation::Native,
        Implementation::Synthesized => wire::PayloadImplementation::Synthesized,
    }) as i32
}

fn frames(value: turret_core::Frames) -> Vec<i32> {
    [
        (value.mount, turret_core::ReferenceFrame::Mount),
        (value.vehicle, turret_core::ReferenceFrame::Vehicle),
        (value.earth_ned, turret_core::ReferenceFrame::EarthNed),
        (
            value.horizon_vehicle_yaw,
            turret_core::ReferenceFrame::HorizonVehicleYaw,
        ),
    ]
    .into_iter()
    .filter_map(|(enabled, frame)| enabled.then_some(super::frame(frame)))
    .collect()
}

fn axis(value: &turret_core::AxisCapabilities) -> wire::PayloadAxisCapabilities {
    let (support, position, reason) = declaration(&value.position);
    let mut position_wire = wire::PayloadPositionCapability {
        support,
        unavailable_reason: reason,
        ..Default::default()
    };
    if let Some(position) = position {
        position_wire.implementation = implementation(position.implementation);
        position_wire.minimum_rad = Some(position.range.minimum.as_radians());
        position_wire.maximum_rad = Some(position.range.maximum.as_radians());
        position_wire.max_rate_rad_s = position
            .maximum_rate
            .map(|rate| rate.as_radians_per_second());
        position_wire.frames = frames(position.frames);
    }
    let (support, rate, reason) = declaration(&value.rate);
    let mut rate_wire = wire::PayloadRateCapability {
        support,
        unavailable_reason: reason,
        ..Default::default()
    };
    if let Some(rate) = rate {
        rate_wire.implementation = implementation(rate.implementation);
        rate_wire.max_rate_rad_s = Some(rate.maximum_rate.as_radians_per_second());
        rate_wire.frames = frames(rate.frames);
    }
    wire::PayloadAxisCapabilities {
        position: Some(position_wire),
        rate: Some(rate_wire),
        angle_feedback: declaration(&value.feedback.angle).0,
        rate_feedback: declaration(&value.feedback.rate).0,
        angle_feedback_unavailable_reason: declaration(&value.feedback.angle).2,
        rate_feedback_unavailable_reason: declaration(&value.feedback.rate).2,
    }
}

fn recenter(
    value: &Capability<turret_core::RecenterCapability>,
) -> wire::PayloadRecenterCapability {
    let (support, target, unavailable_reason) = declaration(value);
    let mut wire = wire::PayloadRecenterCapability {
        support,
        unavailable_reason,
        ..Default::default()
    };
    if let Some(target) = target {
        wire.implementation = implementation(target.implementation);
        if let Some(target) = target.target {
            wire.frame = super::frame(target.frame);
            wire.pitch_rad = target.angles.pitch.map(|angle| angle.as_radians());
            wire.roll_rad = target.angles.roll.map(|angle| angle.as_radians());
            wire.yaw_rad = target.angles.yaw.map(|angle| angle.as_radians());
        }
    }
    wire
}

fn stop(value: turret_core::StopGuarantee) -> (i32, Option<u32>) {
    use turret_core::StopGuarantee as Source;
    use wire::PayloadStopGuarantee as Target;
    let (kind, micros) = match value {
        Source::Unknown => return (Target::Unknown as i32, None),
        Source::BestEffort => return (Target::BestEffort as i32, None),
        Source::FiniteMotion {
            maximum_duration_us,
        } => (Target::FiniteMotion, maximum_duration_us),
        Source::DeviceWatchdog { maximum_silence_us } => {
            (Target::DeviceWatchdog, maximum_silence_us)
        }
    };
    match u32::try_from(micros.div_ceil(1000))
        .ok()
        .filter(|bound| *bound > 0)
    {
        Some(bound) => (kind as i32, Some(bound)),
        None => (Target::Unknown as i32, None),
    }
}

fn zoom(value: &Capability<turret_core::ZoomCapabilities>) -> wire::PayloadZoomCapabilities {
    let (support, capabilities, unavailable_reason) = declaration(value);
    let mut wire = wire::PayloadZoomCapabilities {
        support,
        unavailable_reason,
        ..Default::default()
    };
    let Some(capabilities) = capabilities else {
        return wire;
    };
    wire.absolute = zoom_ranges(&capabilities.absolute);
    wire.feedback_units = zoom_ranges(&capabilities.feedback);
    wire.feedback = aggregate_feedback(&wire.feedback_units);
    let (step, step_capability, step_reason) = declaration(&capabilities.step);
    wire.step = step;
    wire.step_unavailable_reason = step_reason;
    if let Some(step) = step_capability {
        wire.step_implementation = implementation(step.implementation);
        wire.step_maximum_steps = Some(u32::from(step.maximum_steps));
    }
    let (continuous, continuous_implementation, continuous_reason) =
        declaration(&capabilities.continuous);
    wire.continuous = continuous;
    wire.continuous_unavailable_reason = continuous_reason;
    if let Some(value) = continuous_implementation {
        wire.continuous_implementation = implementation(*value);
    }
    (wire.stop_guarantee, wire.stop_bound_ms) = stop(capabilities.stop);
    wire
}

fn zoom_ranges(value: &turret_core::AbsoluteZoomCapabilities) -> Vec<wire::ZoomAbsoluteCapability> {
    [
        turret_core::ZoomUnit::Ratio,
        turret_core::ZoomUnit::TravelFraction,
        turret_core::ZoomUnit::FocalLengthMm,
        turret_core::ZoomUnit::HorizontalFovRadians,
    ]
    .into_iter()
    .map(|unit| {
        let (support, range, unavailable_reason) = declaration(value.get(unit));
        let mut wire = wire::ZoomAbsoluteCapability {
            unit: super::zoom_unit(unit),
            support,
            unavailable_reason,
            ..Default::default()
        };
        if let Some(range) = range {
            wire.implementation = implementation(range.implementation);
            wire.minimum = Some(f64::from(range.minimum.value()));
            wire.maximum = Some(f64::from(range.maximum.value()));
        }
        wire
    })
    .collect()
}

fn aggregate_feedback(values: &[wire::ZoomAbsoluteCapability]) -> i32 {
    use wire::PayloadSupportState as Support;
    for state in [Support::Available, Support::Unavailable, Support::Unknown] {
        if values.iter().any(|value| value.support == state as i32) {
            return state as i32;
        }
    }
    Support::Unsupported as i32
}
