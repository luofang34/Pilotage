//! The heading and altitude path of the flight scope, and the measurement
//! gate that chooses between it and the velocity path.
//!
//! A flight controller that holds heading, altitude, and airspeed with its
//! own sensors flies this intent with no position estimate, so an arm and
//! this intent ask only for a valid attitude. A velocity or an attitude
//! demand is shaped against the measured position and velocity and keeps
//! the full pose gate.

use pilotage_adapter_api::RejectReason;
use pilotage_flight_uplink::HeadingAltitudeDemand;
use pilotage_mavlink::codec::{LocalDownError, local_down_m};
use pilotage_protocol::{ControlIntent, HeadingAltitudeIntent, ScopedControlFrame};

use super::AviateAdapter;
use super::sampling::CurrentPose;
use crate::error::AviateAdapterError;

/// What the host has told the adapter about flying a heading with an
/// altitude. Both start unset: a multirotor host sets neither.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct HeadingAltitudeSettings {
    /// The airspeed bound the flight scope advertises for the intent, m/s.
    /// Unset, the intent is neither advertised nor flown.
    pub(super) max_airspeed_mps: Option<f32>,
    /// The height above the WGS-84 ellipsoid of the flight controller's
    /// local origin, metres. Unset, an altitude above the ellipsoid cannot
    /// be placed in the local frame and is refused.
    pub(super) origin_ellipsoid_height_m: Option<f64>,
}

/// The measurement a flight frame is enacted against.
pub(super) struct ControlMeasurement {
    /// Roll, pitch, and yaw, radians.
    pub(super) attitude_rad: [f32; 3],
    /// The full pose, present when the frame needed it.
    pub(super) pose: Option<CurrentPose>,
}

/// Whether the frame's intent is shaped against the measured position and
/// velocity. A frame that carries only actions, or a heading with an
/// altitude, is not.
fn frame_needs_pose(frame: &ScopedControlFrame) -> bool {
    matches!(
        frame.intent,
        Some(
            ControlIntent::Velocity(_)
                | ControlIntent::AttitudeThrust(_)
                | ControlIntent::PositionHold(_)
                | ControlIntent::BodyRate(_)
                | ControlIntent::GimbalRate(_)
        )
    )
}

impl AviateAdapter {
    /// Lets the flight scope advertise and fly a heading with an altitude,
    /// with `max_airspeed_mps` as the airspeed bound. A host whose flight
    /// controller holds heading, altitude, and airspeed calls it before the
    /// session reads the capabilities.
    ///
    /// # Errors
    ///
    /// [`AviateAdapterError::InvalidHeadingAltitude`] when the bound is not
    /// a finite number greater than zero.
    pub fn enable_heading_altitude(
        &mut self,
        max_airspeed_mps: f32,
    ) -> Result<(), AviateAdapterError> {
        if !(max_airspeed_mps.is_finite() && max_airspeed_mps > 0.0) {
            return Err(AviateAdapterError::InvalidHeadingAltitude {
                detail: format!("the airspeed bound {max_airspeed_mps} m/s is not positive"),
            });
        }
        self.heading_altitude.max_airspeed_mps = Some(max_airspeed_mps);
        Ok(())
    }

    /// Sets the height above the WGS-84 ellipsoid of the flight
    /// controller's local origin: the number the host gave the flight
    /// controller as its origin. The flight controller's own origin report
    /// cannot supply it, because its altitude is above mean sea level.
    ///
    /// # Errors
    ///
    /// [`AviateAdapterError::InvalidHeadingAltitude`] for a height that is
    /// not finite.
    pub fn set_local_origin_height(&mut self, ellipsoid_m: f64) -> Result<(), AviateAdapterError> {
        if !ellipsoid_m.is_finite() {
            return Err(AviateAdapterError::InvalidHeadingAltitude {
                detail: format!("the local origin height {ellipsoid_m} m is not finite"),
            });
        }
        self.heading_altitude.origin_ellipsoid_height_m = Some(ellipsoid_m);
        Ok(())
    }

    /// The measurement `frame` is enacted against: the full pose for an
    /// intent shaped against position and velocity, the attitude alone
    /// otherwise. `None` when that measurement is not available.
    pub(super) fn control_measurement(
        &mut self,
        frame: &ScopedControlFrame,
    ) -> Option<ControlMeasurement> {
        if frame_needs_pose(frame) {
            let pose = self.current_pose()?;
            return Some(ControlMeasurement {
                attitude_rad: pose.attitude_rad,
                pose: Some(pose),
            });
        }
        Some(ControlMeasurement {
            attitude_rad: self.current_attitude()?,
            pose: None,
        })
    }
}

/// The local-frame demand of `intent` under `settings`.
///
/// # Errors
///
/// The reason the intent cannot be flown: the host has not enabled it, an
/// altitude above the ellipsoid arrived before the origin height, or the
/// down coordinate is not a finite number.
pub(super) fn heading_altitude_demand(
    intent: &HeadingAltitudeIntent,
    settings: HeadingAltitudeSettings,
) -> Result<HeadingAltitudeDemand, RejectReason> {
    if settings.max_airspeed_mps.is_none() {
        return Err(RejectReason::Other(
            "this vehicle does not fly a heading with an altitude".to_owned(),
        ));
    }
    let down_m =
        local_down_m(intent, settings.origin_ellipsoid_height_m).map_err(|error| match error {
            LocalDownError::OriginHeightUnknown { .. } => RejectReason::LocalOriginHeightUnknown,
            LocalDownError::NotFinite { .. } => RejectReason::Other(error.to_string()),
        })?;
    Ok(HeadingAltitudeDemand {
        down_m,
        heading_rad: intent.heading_rad,
        airspeed_mps: intent.airspeed_mps,
    })
}

#[cfg(test)]
mod tests;
