//! Validation of payload commands against declared capabilities.

use crate::{ZoomCommand, wire};

/// The available angular rate limit, or no usable rate control.
#[must_use]
pub fn payload_rate_limit(axis: Option<&wire::PayloadAxisCapabilities>) -> Option<f32> {
    let rate = axis?.rate.as_ref()?;
    let limit = rate.max_rate_rad_s?;
    (rate.support == wire::PayloadSupportState::Available as i32
        && limit.is_finite()
        && limit > 0.0)
        .then_some(limit)
}

/// Whether the camera advertises this exact operation and unit.
#[must_use]
pub fn payload_zoom_admits(
    capabilities: Option<&wire::PayloadZoomCapabilities>,
    command: ZoomCommand,
) -> bool {
    let Some(capabilities) = capabilities else {
        return false;
    };
    let available = wire::PayloadSupportState::Available as i32;
    if capabilities.support != available {
        return false;
    }
    match command {
        ZoomCommand::Step(steps) => {
            steps != 0
                && capabilities.step == available
                && capabilities
                    .step_maximum_steps
                    .is_some_and(|maximum| u32::from(steps.unsigned_abs()) <= maximum)
        }
        ZoomCommand::Continuous(rate) => {
            capabilities.continuous == available
                && (-1_000_000..=1_000_000).contains(&rate)
                && (rate == 0
                    || (matches!(
                        wire::PayloadStopGuarantee::try_from(capabilities.stop_guarantee),
                        Ok(wire::PayloadStopGuarantee::FiniteMotion
                            | wire::PayloadStopGuarantee::DeviceWatchdog)
                    ) && capabilities.stop_bound_ms.is_some_and(|bound| bound > 0)))
        }
        ZoomCommand::Absolute { unit, value_micros } => {
            let value = value_micros as f64 / 1_000_000.0;
            capabilities.absolute.iter().any(|capability| {
                capability.unit == unit.to_wire() as i32
                    && capability.support == available
                    && capability.minimum.zip(capability.maximum).is_some_and(
                        |(minimum, maximum)| {
                            minimum.is_finite()
                                && maximum.is_finite()
                                && (minimum..=maximum).contains(&value)
                        },
                    )
            })
        }
    }
}
