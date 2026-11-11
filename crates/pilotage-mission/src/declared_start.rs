//! The operator's statement of where and how the vehicle starts, for a
//! navigator that has no satellite fix to start from.
//!
//! These are statements only, each with the operator's own 1-sigma: no
//! attitude is assembled from them and no covariance is built here. The
//! navigator owns its initial state; the mission hands it what the
//! operator said and nothing derived from it.

use navigate_contract::MonotonicNanos;
use pilotage_geo::VerticalDatum;

/// Where the operator says the vehicle is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeclaredPosition {
    /// Latitude on WGS-84, radians.
    pub latitude_rad: f64,
    /// Longitude on WGS-84, radians.
    pub longitude_rad: f64,
    /// 1-sigma horizontal uncertainty, metres.
    pub sigma_m: f64,
}

/// The height the operator states, against the datum they state it on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeclaredHeight {
    /// The height, metres, positive up.
    pub height_m: f64,
    /// What the height is measured from. A height on a datum that needs a
    /// model to reach the ellipsoid (sea level, ground) is converted by the
    /// navigator with its own model, not here.
    pub datum: VerticalDatum,
    /// 1-sigma uncertainty, metres.
    pub sigma_m: f64,
}

/// The true heading the operator states.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeclaredHeading {
    /// True heading, radians in `[-π, π]`: zero north, positive toward
    /// east.
    pub heading_rad: f64,
    /// 1-sigma uncertainty, radians.
    pub sigma_rad: f64,
}

/// What a declared speed is measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedReference {
    /// Speed through the air, indicated.
    Airspeed,
    /// Speed over the ground.
    GroundSpeed,
}

/// The speed the operator states.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeclaredSpeed {
    /// The speed, m/s, zero or more.
    pub speed_mps: f64,
    /// What the speed is measured against.
    pub reference: SpeedReference,
    /// 1-sigma uncertainty, m/s.
    pub sigma_mps: f64,
}

/// The operator's statement of the start. Each part is optional: an
/// operator states what they know.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeclaredStart {
    /// Where the vehicle is.
    pub position: Option<DeclaredPosition>,
    /// Its height.
    pub height: Option<DeclaredHeight>,
    /// Its true heading.
    pub heading: Option<DeclaredHeading>,
    /// Its speed.
    pub speed: Option<DeclaredSpeed>,
    /// When the operator made the statement, on the mission's clock domain
    /// ([`crate::MissionConfig::clock`]).
    pub declared_at: MonotonicNanos,
}

/// Why a declared start is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the declared start's {field} is not a statement a navigator can use")]
pub struct DeclaredStartError {
    /// The refused part, as a dotted field name.
    pub field: &'static str,
}

impl DeclaredStart {
    /// Refuses a statement no navigator can take: a value that is not a
    /// finite number, a 1-sigma that is not greater than zero, a position
    /// off the Earth, a heading outside one turn, a negative speed, or a
    /// height on a datum this build does not know.
    ///
    /// # Errors
    ///
    /// [`DeclaredStartError`] naming the first refused field.
    pub fn validate(&self) -> Result<(), DeclaredStartError> {
        let refuse = |ok: bool, field| {
            if ok {
                Ok(())
            } else {
                Err(DeclaredStartError { field })
            }
        };
        let sigma = |value: f64| value.is_finite() && value > 0.0;
        if let Some(position) = self.position {
            refuse(
                position.latitude_rad.is_finite()
                    && position.latitude_rad.abs() <= core::f64::consts::FRAC_PI_2,
                "position.latitude_rad",
            )?;
            refuse(
                position.longitude_rad.is_finite()
                    && position.longitude_rad.abs() <= core::f64::consts::PI,
                "position.longitude_rad",
            )?;
            refuse(sigma(position.sigma_m), "position.sigma_m")?;
        }
        if let Some(height) = self.height {
            refuse(height.height_m.is_finite(), "height.height_m")?;
            refuse(height.datum != VerticalDatum::Unknown, "height.datum")?;
            refuse(sigma(height.sigma_m), "height.sigma_m")?;
        }
        if let Some(heading) = self.heading {
            refuse(
                heading.heading_rad.is_finite()
                    && heading.heading_rad.abs() <= core::f64::consts::PI,
                "heading.heading_rad",
            )?;
            refuse(sigma(heading.sigma_rad), "heading.sigma_rad")?;
        }
        if let Some(speed) = self.speed {
            refuse(
                speed.speed_mps.is_finite() && speed.speed_mps >= 0.0,
                "speed.speed_mps",
            )?;
            refuse(sigma(speed.sigma_mps), "speed.sigma_mps")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
