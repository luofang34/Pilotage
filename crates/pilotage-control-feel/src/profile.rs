//! Control-feel profile data.

use serde::{Deserialize, Serialize};

/// The supported control-feel schema version.
pub const SCHEMA_VERSION: u16 = 1;

/// An operator control-feel mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeelMode {
    /// Low center gain and low command jerk.
    Precision,
    /// The default response and smoothness balance.
    Balanced,
    /// Faster response within the same vehicle safety envelope.
    Agile,
    /// Compatibility with the command law that precedes this schema.
    LegacyCompatibility,
}

/// Full-input operator demand limits.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemandEnvelope {
    /// Maximum horizontal speed demand in m/s.
    pub horizontal_speed_mps: f32,
    /// Maximum climb or descent speed demand in m/s.
    pub vertical_speed_mps: f32,
    /// Maximum yaw-rate demand in rad/s.
    pub yaw_rate_rps: f32,
    /// Maximum direct roll or pitch demand in rad.
    pub direct_tilt_rad: f32,
    /// Direct-mode thrust at a centered collective axis.
    pub direct_hover_thrust: f32,
    /// Direct-mode thrust at minimum collective.
    pub direct_min_thrust: f32,
    /// Normalized climb input that opens the takeoff stream.
    pub takeoff_input: f32,
}

/// A monotonic signed response curve.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxisCurve {
    /// Curve exponent offset. Zero gives a linear response.
    pub expo: f32,
}

impl AxisCurve {
    /// Apply the curve to a normalized input.
    #[must_use]
    pub fn apply(self, value: f32) -> f32 {
        if !value.is_finite() {
            return 0.0;
        }
        let bounded = value.clamp(-1.0, 1.0);
        let exponent = 1.0 + self.expo.clamp(0.0, 0.8);
        bounded.signum() * bounded.abs().powf(exponent)
    }
}

/// Hysteresis for active and neutral input states.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeutralBand {
    /// Magnitude that changes a neutral input to active.
    pub active_enter: f32,
    /// Magnitude that changes an active input to neutral.
    pub active_exit: f32,
}

/// Time-domain limits for one demand axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxisDynamics {
    /// Maximum demand acceleration while input is active.
    pub apply_accel: f32,
    /// Maximum demand acceleration while input is neutral.
    pub release_accel: f32,
    /// Maximum demand jerk while input is active.
    pub apply_jerk: f32,
    /// Maximum demand jerk while input is neutral.
    pub release_jerk: f32,
}

/// Curve, hysteresis, and time response for one demand family.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxisResponse {
    /// Static response curve.
    pub curve: AxisCurve,
    /// Active-state hysteresis.
    pub neutral: NeutralBand,
    /// Apply and release time limits.
    pub dynamics: AxisDynamics,
}

/// Direct attitude and thrust time limits.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectDynamics {
    /// Maximum direct attitude change in rad/s.
    pub tilt_rate_rps: f32,
    /// Maximum direct attitude acceleration in rad/s².
    pub tilt_accel_rps2: f32,
    /// Maximum normalized thrust change per second.
    pub thrust_rate_per_s: f32,
    /// Maximum normalized thrust acceleration per second².
    pub thrust_accel_per_s2: f32,
}

/// Conditions that prove the brake phase is stable.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoldTransition {
    /// Maximum measured speed for a stable sample in m/s.
    pub max_speed_mps: f32,
    /// Maximum measured acceleration for a stable sample in m/s².
    pub max_accel_mps2: f32,
    /// Require a valid acceleration sample before capture.
    pub require_accel: bool,
    /// Required stable interval in milliseconds.
    pub stable_dwell_ms: u32,
}

/// One complete operator control-feel artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightFeelProfile {
    /// Schema version. This value must equal [`SCHEMA_VERSION`].
    pub schema_version: u16,
    /// Stable profile name for logs and operator selection.
    pub profile_id: String,
    /// Named operator mode.
    pub mode: FeelMode,
    /// Full-input demand limits.
    pub envelope: DemandEnvelope,
    /// Horizontal demand response.
    pub horizontal: AxisResponse,
    /// Vertical demand response.
    pub vertical: AxisResponse,
    /// Yaw-rate demand response.
    pub yaw: AxisResponse,
    /// Direct attitude and thrust response.
    pub direct: DirectDynamics,
    /// Brake-to-hold transition conditions.
    pub hold: HoldTransition,
}

impl FlightFeelProfile {
    /// Return the fixed compatibility profile.
    #[must_use]
    pub fn legacy_compatibility() -> Self {
        let axis = AxisResponse {
            curve: AxisCurve { expo: 0.0 },
            neutral: NeutralBand {
                active_enter: 0.02,
                active_exit: 0.02,
            },
            dynamics: AxisDynamics {
                apply_accel: 5.0,
                release_accel: 10_000.0,
                apply_jerk: 100_000.0,
                release_jerk: 100_000.0,
            },
        };
        Self {
            schema_version: SCHEMA_VERSION,
            profile_id: "alia250-legacy-v1".to_owned(),
            mode: FeelMode::LegacyCompatibility,
            envelope: DemandEnvelope {
                horizontal_speed_mps: 3.0,
                vertical_speed_mps: 1.5,
                yaw_rate_rps: 0.9,
                direct_tilt_rad: 0.6,
                direct_hover_thrust: 0.72,
                direct_min_thrust: 0.30,
                takeoff_input: 0.15,
            },
            horizontal: axis,
            vertical: AxisResponse {
                dynamics: AxisDynamics {
                    apply_accel: 10_000.0,
                    ..axis.dynamics
                },
                ..axis
            },
            yaw: AxisResponse {
                dynamics: AxisDynamics {
                    apply_accel: 10_000.0,
                    ..axis.dynamics
                },
                ..axis
            },
            direct: DirectDynamics {
                tilt_rate_rps: 10_000.0,
                tilt_accel_rps2: 100_000.0,
                thrust_rate_per_s: 10_000.0,
                thrust_accel_per_s2: 100_000.0,
            },
            hold: HoldTransition {
                max_speed_mps: 0.3,
                max_accel_mps2: 10_000.0,
                require_accel: false,
                stable_dwell_ms: 0,
            },
        }
    }
}

impl Default for FlightFeelProfile {
    fn default() -> Self {
        Self::legacy_compatibility()
    }
}
