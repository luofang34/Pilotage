//! The fixed-wing airframe of the adapter: its construction, the bounds of
//! its velocity demand, the heading and altitude path, the check of the
//! flight controller's reported airframe, and the measurement gate that
//! chooses between attitude alone and the full pose.
//!
//! A fixed wing's flight controller holds a course or a heading, an
//! altitude or a climb rate, and an airspeed with its own sensors, so its
//! demands bypass the multirotor stick law. An arm and a heading with an
//! altitude ask only for a valid attitude; a velocity or an attitude
//! demand keeps the full pose gate.

use pilotage_adapter_api::RejectReason;
use pilotage_control_feel::{
    FixedWingEnvelope, FlightFeelProfile, ValidatedFixedWingEnvelope, ValidatedFlightFeelProfile,
};
use pilotage_flight_uplink::HeadingAltitudeDemand;
use pilotage_mavlink::codec::{LocalDownError, local_down_m};
use pilotage_mavlink::link::LinkConfig;
use pilotage_protocol::{
    ControlIntent, HeadingAltitudeIntent, ScopedControlFrame, VehicleId, VelocityIntent,
};
use web_time::Instant;

use super::sampling::CurrentPose;
use super::{AviateAdapter, EmbeddedIncarnations};
use crate::error::AviateAdapterError;

/// The airframe an embedded adapter flies, with the law its demands are
/// bounded by.
#[derive(Debug, Clone, PartialEq)]
pub enum AviateAirframe {
    /// A multirotor: operator sticks shaped by a control-feel law.
    Multirotor(Box<ValidatedFlightFeelProfile>),
    /// A fixed wing: demands bounded by its envelope and flown by the
    /// flight controller's own fixed-wing law.
    FixedWing(ValidatedFixedWingEnvelope),
}

/// What the host has said about a fixed wing. Both start unset: a
/// multirotor sets neither.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct FixedWingSettings {
    /// The envelope, when the adapter flies a fixed wing.
    pub(super) envelope: Option<ValidatedFixedWingEnvelope>,
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

/// What a MAV_TYPE says about the airframe: `Some(true)` for a fixed wing,
/// `Some(false)` for a rotorcraft, `None` for a type that names neither
/// (generic, a ground station, a VTOL that is both).
const fn reports_fixed_wing(mav_type: u8) -> Option<bool> {
    match mav_type {
        1 => Some(true),
        // Coaxial, helicopter, quad, hexa, octo, tri, dodeca, deca rotors.
        2 | 3 | 4 | 13 | 14 | 15 | 29 | 35 => Some(false),
        _ => None,
    }
}

impl AviateAdapter {
    /// An embedded adapter for `airframe`; see [`Self::embedded`]. A fixed
    /// wing advertises [`pilotage_protocol::VehicleClass::FixedWing`], a
    /// velocity bounded by its envelope, and a heading with an altitude,
    /// and offers no control-feel law: there is no stick law to shape.
    ///
    /// # Errors
    ///
    /// Returns [`AviateAdapterError`] when a multirotor's control-feel
    /// profile does not match this adapter.
    pub fn embedded_for(
        vehicle: VehicleId,
        config: LinkConfig,
        airframe: AviateAirframe,
        incarnations: EmbeddedIncarnations,
        now: Instant,
    ) -> Result<Self, AviateAdapterError> {
        let envelope = match airframe {
            AviateAirframe::Multirotor(feel) => {
                return Self::embedded(vehicle, config, *feel, incarnations, now);
            }
            AviateAirframe::FixedWing(envelope) => envelope,
        };
        // The uplink numbers frames, keeps the post-arm quiet interval, and
        // repeats the link-loss hold with the compatibility law; no
        // fixed-wing demand passes through its stick shaping.
        let sequencing = ValidatedFlightFeelProfile::new(FlightFeelProfile::legacy_compatibility())
            .map_err(|source| AviateAdapterError::InvalidControlFeel { source })?;
        let mut adapter = Self::embedded(vehicle, config, sequencing, incarnations, now)?;
        adapter.control_feel_profiles = None;
        adapter.fixed_wing.envelope = Some(envelope);
        Ok(adapter)
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
        self.fixed_wing.origin_ellipsoid_height_m = Some(ellipsoid_m);
        Ok(())
    }

    /// The fixed-wing envelope, when the adapter flies a fixed wing.
    pub(super) fn fixed_wing_envelope(&self) -> Option<FixedWingEnvelope> {
        self.fixed_wing
            .envelope
            .map(|envelope| *envelope.envelope())
    }

    /// The airframe the flight controller reports, when it contradicts the
    /// one this adapter flies. A heartbeat not yet seen, or a type that
    /// names neither airframe, contradicts nothing.
    pub(super) fn airframe_contradiction(&self) -> Option<RejectReason> {
        let reported = self
            .uplink
            .as_ref()
            .and_then(crate::uplink::FlightUplink::fc_mav_type)
            .or_else(|| {
                let source = self.estimate.as_ref()?;
                source.state.lock().ok()?.fc_reports.mav_type
            })?;
        let flies_fixed_wing = self.fixed_wing.envelope.is_some();
        (reports_fixed_wing(reported)? != flies_fixed_wing).then_some(
            RejectReason::AirframeMismatch {
                reported_mav_type: reported,
            },
        )
    }

    /// What the airframe settles before any action rides `frame`, so an arm
    /// is not sent beside a demand the adapter then refuses: an airframe the
    /// flight controller contradicts refuses every frame, and a heading with
    /// an altitude becomes its local-frame demand.
    pub(super) fn airframe_preconditions(
        &self,
        frame: &ScopedControlFrame,
    ) -> Result<Option<HeadingAltitudeDemand>, RejectReason> {
        if let Some(reason) = self.airframe_contradiction() {
            return Err(reason);
        }
        match &frame.intent {
            Some(ControlIntent::HeadingAltitude(intent)) => {
                heading_altitude_demand(intent, self.fixed_wing).map(Some)
            }
            _ => Ok(None),
        }
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
/// The reason the intent cannot be flown: the adapter flies no fixed wing,
/// an altitude above the ellipsoid arrived before the origin height, or
/// the down coordinate is not a finite number.
fn heading_altitude_demand(
    intent: &HeadingAltitudeIntent,
    settings: FixedWingSettings,
) -> Result<HeadingAltitudeDemand, RejectReason> {
    if settings.envelope.is_none() {
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

/// The local NED velocity of a body-frame `velocity` at `yaw_rad`, bounded
/// by `envelope`, and whether a bound changed it.
///
/// A horizontal velocity with a direction is a course and an airspeed: its
/// magnitude is brought into the airspeed range, never its direction. One
/// with no direction asks the flight controller to hold the heading at its
/// cruise airspeed and is passed as it is. The vertical component is
/// bounded by the climb rate upward and the sink rate downward.
pub(super) fn fixed_wing_velocity(
    velocity: &VelocityIntent,
    yaw_rad: f32,
    envelope: &FixedWingEnvelope,
) -> ([f32; 3], bool) {
    let (sin, cos) = f64::from(yaw_rad).sin_cos();
    let (forward, right) = (f64::from(velocity.vx), f64::from(velocity.vy));
    let north = forward * cos - right * sin;
    let east = forward * sin + right * cos;
    let speed = north.hypot(east);
    let bounded = speed.clamp(envelope.min_airspeed_mps, envelope.max_airspeed_mps);
    let (north, east, horizontal_constrained) = if speed > 0.0 && speed.is_finite() {
        let scale = bounded / speed;
        (north * scale, east * scale, bounded != speed)
    } else {
        (0.0, 0.0, speed != 0.0)
    };
    let down = f64::from(velocity.vz);
    let bounded_down = if down.is_finite() {
        down.clamp(-envelope.max_climb_mps, envelope.max_sink_mps)
    } else {
        0.0
    };
    (
        [north as f32, east as f32, bounded_down as f32],
        horizontal_constrained || bounded_down != down,
    )
}

#[cfg(test)]
mod tests;
