use serde::{Deserialize, Serialize};

use crate::error::XPlaneTrialError;
use crate::protocol::finite_number;

/// One causal X-Plane truth sample from the trial plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct XPlaneTruthSample {
    /// The active trial generation.
    pub generation: u64,
    /// The zero-based sample sequence.
    pub sequence: u64,
    /// Simulator time since the current flight loaded.
    pub sim_time_s: f64,
    /// Simulator time since this trial started.
    pub trial_time_s: f64,
    /// The observed simulator reset generation.
    pub reset_generation: u64,
    /// X-Plane local position coordinates.
    pub local_position_m: [f64; 3],
    /// X-Plane local velocity coordinates.
    pub local_velocity_mps: [f64; 3],
    /// X-Plane kinematic acceleration in local coordinates.
    ///
    /// This value is not an accelerometer specific-force or load-factor
    /// measurement.
    pub local_acceleration_mps2: [f64; 3],
    /// Body specific force in forward, right, and down axes, in g.
    pub body_specific_force_g: [f64; 3],
    /// X-Plane aircraft quaternion in `{1, i, j, k}` order.
    pub quaternion: [f64; 4],
    /// Body roll, pitch, and yaw rates in radians per second.
    pub body_rates_rps: [f64; 3],
    /// Ground-contact state, or `None` when the dataref is absent.
    pub on_ground: Option<bool>,
    /// Crash state, or `None` when the dataref is absent.
    pub crashed: Option<bool>,
    /// Actual local wind speed in meters per second.
    pub wind_speed_mps: f64,
    /// Actual local wind direction in degrees true.
    pub wind_direction_deg: f64,
}

impl XPlaneTruthSample {
    /// Returns position in north-east-down coordinates.
    #[must_use]
    pub fn position_ned_m(&self) -> [f64; 3] {
        local_to_ned(self.local_position_m)
    }

    /// Returns velocity in north-east-down coordinates.
    #[must_use]
    pub fn velocity_ned_mps(&self) -> [f64; 3] {
        local_to_ned(self.local_velocity_mps)
    }

    /// Returns acceleration in north-east-down coordinates.
    #[must_use]
    pub fn acceleration_ned_mps2(&self) -> [f64; 3] {
        local_to_ned(self.local_acceleration_mps2)
    }

    pub(crate) fn parse_fields(fields: &[&str]) -> Result<Self, XPlaneTrialError> {
        if fields.len() != 29 {
            return invalid("SAMPLE has an invalid field count");
        }
        let sample = Self {
            generation: unsigned(fields[1], "sample generation")?,
            sequence: unsigned(fields[2], "sample sequence")?,
            sim_time_s: finite_number(fields, 3, "sample simulator time")?,
            trial_time_s: finite_number(fields, 4, "sample trial time")?,
            reset_generation: unsigned(fields[5], "sample reset generation")?,
            local_position_m: triple(fields, 6, "local position")?,
            local_velocity_mps: triple(fields, 9, "local velocity")?,
            local_acceleration_mps2: triple(fields, 12, "local acceleration")?,
            body_specific_force_g: triple(fields, 15, "body specific force")?,
            quaternion: quadruple(fields, 18, "quaternion")?,
            body_rates_rps: triple(fields, 22, "body rates")?,
            on_ground: optional_bool(fields[25], "ground contact")?,
            crashed: optional_bool(fields[26], "crash state")?,
            wind_speed_mps: finite_number(fields, 27, "wind speed")?,
            wind_direction_deg: finite_number(fields, 28, "wind direction")?,
        };
        sample.validate()?;
        Ok(sample)
    }

    fn validate(&self) -> Result<(), XPlaneTrialError> {
        if self.generation == 0 {
            return invalid("sample generation is zero");
        }
        if self.sim_time_s < 0.0 || self.trial_time_s < 0.0 {
            return invalid("sample time is negative");
        }
        if self.wind_speed_mps < 0.0 || !(0.0..=360.0).contains(&self.wind_direction_deg) {
            return invalid("sample wind is outside its range");
        }
        let norm = self
            .quaternion
            .iter()
            .map(|value| value * value)
            .sum::<f64>();
        if !(0.5..=1.5).contains(&norm) {
            return invalid("sample quaternion norm is outside its range");
        }
        Ok(())
    }
}

fn local_to_ned(value: [f64; 3]) -> [f64; 3] {
    [-value[2], value[0], -value[1]]
}

fn triple(fields: &[&str], start: usize, name: &str) -> Result<[f64; 3], XPlaneTrialError> {
    Ok([
        finite_number(fields, start, name)?,
        finite_number(fields, start + 1, name)?,
        finite_number(fields, start + 2, name)?,
    ])
}

fn quadruple(fields: &[&str], start: usize, name: &str) -> Result<[f64; 4], XPlaneTrialError> {
    Ok([
        finite_number(fields, start, name)?,
        finite_number(fields, start + 1, name)?,
        finite_number(fields, start + 2, name)?,
        finite_number(fields, start + 3, name)?,
    ])
}

fn unsigned(value: &str, field: &str) -> Result<u64, XPlaneTrialError> {
    value
        .parse::<u64>()
        .map_err(|_| invalid_value(format!("{field} is invalid")))
}

fn optional_bool(value: &str, field: &str) -> Result<Option<bool>, XPlaneTrialError> {
    match value {
        "-1" => Ok(None),
        "0" => Ok(Some(false)),
        "1" => Ok(Some(true)),
        _ => invalid(format!("{field} is invalid")),
    }
}

fn invalid<T>(detail: impl Into<String>) -> Result<T, XPlaneTrialError> {
    Err(invalid_value(detail))
}

fn invalid_value(detail: impl Into<String>) -> XPlaneTrialError {
    XPlaneTrialError::InvalidProtocol {
        detail: detail.into(),
    }
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::local_to_ned;

    #[test]
    fn xplane_local_axes_map_to_north_east_down() {
        assert_eq!(local_to_ned([2.0, 3.0, 5.0]), [-5.0, 2.0, -3.0]);
    }
}
