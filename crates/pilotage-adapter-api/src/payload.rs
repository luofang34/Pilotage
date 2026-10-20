//! Lossless projection of portable Turret contracts into Pilotage payload types.

mod capabilities;
mod observation;
mod progress;

pub use capabilities::payload_capabilities_to_wire;
pub use observation::{
    payload_angle_observation_to_wire, payload_rate_observation_to_wire,
    payload_zoom_observation_to_wire,
};
pub use progress::payload_progress_to_wire;

use pilotage_protocol::wire;

fn frame(value: turret_core::ReferenceFrame) -> i32 {
    (match value {
        turret_core::ReferenceFrame::Mount => wire::PayloadReferenceFrame::Mount,
        turret_core::ReferenceFrame::Vehicle => wire::PayloadReferenceFrame::BodyFrd,
        turret_core::ReferenceFrame::EarthNed => wire::PayloadReferenceFrame::LocalNed,
        turret_core::ReferenceFrame::HorizonVehicleYaw => {
            wire::PayloadReferenceFrame::HorizonVehicleYaw
        }
    }) as i32
}

fn timestamp(tick: turret_core::Tick) -> Option<wire::MonoTimestamp> {
    tick.0
        .checked_mul(1000)
        .map(|nanos| wire::MonoTimestamp { nanos })
}

fn zoom_unit(value: turret_core::ZoomUnit) -> i32 {
    (match value {
        turret_core::ZoomUnit::Ratio => wire::ZoomUnit::Ratio,
        turret_core::ZoomUnit::TravelFraction => wire::ZoomUnit::TravelFraction,
        turret_core::ZoomUnit::FocalLengthMm => wire::ZoomUnit::FocalLengthMm,
        turret_core::ZoomUnit::HorizontalFovRadians => wire::ZoomUnit::HorizontalFovRadians,
    }) as i32
}

#[cfg(test)]
mod tests;
