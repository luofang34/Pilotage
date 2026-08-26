//! Control-feel profile data.

use serde::{Deserialize, Serialize};

use crate::ProfileBindings;

const LEGACY_DEVICE_PROFILE_SHA256: [u8; 32] = [
    0x32, 0x85, 0x73, 0x85, 0x65, 0x47, 0xb1, 0x64, 0x6e, 0xca, 0xe8, 0x74, 0x38, 0x15, 0xbe, 0x16,
    0x1d, 0x5a, 0xba, 0x9b, 0x97, 0x4a, 0xaa, 0xfd, 0xf9, 0x75, 0x6c, 0xe3, 0x04, 0x6d, 0x0d, 0x17,
];
const LEGACY_FLIGHT_CONTROLLER_SHA256: [u8; 32] = [
    0x06, 0x69, 0xf5, 0x34, 0x45, 0x32, 0xba, 0xe5, 0xff, 0x81, 0x71, 0x93, 0xe0, 0xec, 0xbd, 0x33,
    0x67, 0xe2, 0x35, 0xa4, 0x1d, 0x06, 0x44, 0x36, 0xfc, 0xff, 0x45, 0xe2, 0x7c, 0xed, 0x55, 0xcd,
];

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
    /// Input magnitude at which the curve starts.
    pub deadzone: f32,
    /// Exponent offset near the center. Zero gives a linear response.
    pub center_expo: f32,
    /// Exponent offset near full input. Zero gives a linear response.
    pub outer_expo: f32,
    /// Input magnitude at which the outer curve starts to blend.
    pub outer_start: f32,
}

impl AxisCurve {
    /// Apply the curve to a normalized input.
    #[must_use]
    pub fn apply(self, value: f32) -> f32 {
        if !value.is_finite() {
            return 0.0;
        }
        let bounded = value.clamp(-1.0, 1.0);
        let deadzone = self.deadzone.clamp(0.0, 1.0 - f32::EPSILON);
        let magnitude = bounded.abs();
        if magnitude <= deadzone {
            return 0.0;
        }
        let scaled = (magnitude - deadzone) / (1.0 - deadzone);
        let center = scaled.powf(1.0 + self.center_expo);
        let outer = scaled.powf(1.0 + self.outer_expo);
        let blend = if self.outer_start >= 1.0 {
            0.0
        } else {
            ((scaled - self.outer_start) / (1.0 - self.outer_start)).clamp(0.0, 1.0)
        };
        bounded.signum() * (center + (outer - center) * blend)
    }
}

/// Hysteresis for curved demand and neutral states.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeutralBand {
    /// Curved magnitude that changes a neutral input to active.
    pub active_enter: f32,
    /// Curved magnitude that changes an active input to neutral.
    pub active_exit: f32,
    /// Continuous neutral interval before release, in milliseconds.
    pub dwell_ms: u32,
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
    /// Maximum demand acceleration during a direction reversal.
    pub reversal_accel: f32,
    /// Maximum demand jerk during a direction reversal.
    pub reversal_jerk: f32,
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
    /// Artifact identities required by this profile.
    pub bindings: ProfileBindings,
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
    /// Return the shaped starting profile for one operator mode.
    ///
    /// These are starting points, not qualified calibrations. A qualified
    /// default comes out of a tuning campaign that measures this vehicle;
    /// what these give it is a safe, principled place to start from, and what
    /// they give an operator today is a command law that does not step.
    ///
    /// Two sets of principles shape them, and both are checked rather than
    /// described:
    ///
    /// An electric vehicle delivers torque progressively. Demand is bounded in
    /// acceleration and in jerk, so no input produces a step; lifting off is a
    /// controlled ramp rather than an instant return to zero, which is what
    /// makes a release predictable instead of a lurch. It is a PROMPT ramp:
    /// letting go is how an operator stops asking, and a release that lagged
    /// the apply would take longer to stop commanding than to start. A neutral
    /// band with a dwell keeps a resting hand from commanding, and the band is
    /// hysteretic so an input sitting on its edge does not chatter between
    /// commanding and not.
    ///
    /// A control surface has to be predictable and consistent. The three modes
    /// differ in degree and never in kind: the same curve family, the same
    /// ordering of limits, the same structure of band. An operator who has
    /// learned one knows what the others will do. Response begins on the first
    /// sample rather than after a delay, because a control that does not
    /// answer immediately reads as broken however smoothly it moves later.
    ///
    /// [`FeelMode::LegacyCompatibility`] returns [`Self::legacy_compatibility`],
    /// which is the unshaped command law and is deliberately none of this.
    #[must_use]
    pub fn shaped(mode: FeelMode) -> Self {
        let tuning = match mode {
            FeelMode::LegacyCompatibility => return Self::legacy_compatibility(),
            FeelMode::Precision => ModeTuning {
                deadzone: 0.08,
                center_expo: 0.50,
                outer_expo: 0.35,
                enter: 0.045,
                exit: 0.030,
                dwell_ms: 120,
                apply_accel: 2.5,
                apply_jerk: 12.0,
            },
            FeelMode::Balanced => ModeTuning {
                deadzone: 0.06,
                center_expo: 0.35,
                outer_expo: 0.30,
                enter: 0.035,
                exit: 0.022,
                dwell_ms: 90,
                apply_accel: 4.0,
                apply_jerk: 24.0,
            },
            FeelMode::Agile => ModeTuning {
                deadzone: 0.04,
                center_expo: 0.20,
                outer_expo: 0.20,
                enter: 0.028,
                exit: 0.018,
                dwell_ms: 60,
                apply_accel: 6.5,
                apply_jerk: 45.0,
            },
        };
        let legacy = Self::legacy_compatibility();
        Self {
            profile_id: format!("alia250-shaped-{}-v1", tuning.slug(mode)),
            mode,
            horizontal: tuning.axis(1.0),
            // Altitude is the axis a passenger feels most directly, so it is
            // held to a gentler bound than the horizontal one at every mode.
            vertical: tuning.axis(0.7),
            yaw: tuning.axis(0.85),
            // Direct flight is a family of its own, and leaving it on the
            // compatibility law would give an operator three modes that are
            // the same law: byte-identical, and stepping. The attitude command
            // is what the stick moves in this family, so it is bounded and
            // jerk-limited for the same reason the velocity families are.
            direct: tuning.direct(),
            // The brake-to-hold transition has to prove the vehicle is
            // actually stable, which a zero dwell cannot: one sample under the
            // ceiling is a moment, not a state.
            hold: tuning.hold(),
            ..legacy
        }
    }

    /// Return the fixed compatibility profile.
    #[must_use]
    pub fn legacy_compatibility() -> Self {
        let axis = AxisResponse {
            curve: AxisCurve {
                deadzone: 0.06,
                center_expo: 0.35,
                outer_expo: 0.35,
                outer_start: 1.0,
            },
            neutral: NeutralBand {
                active_enter: 0.02,
                active_exit: 0.02,
                dwell_ms: 0,
            },
            dynamics: AxisDynamics {
                apply_accel: 5.0,
                release_accel: 10_000.0,
                apply_jerk: 100_000.0,
                release_jerk: 100_000.0,
                reversal_accel: 10_000.0,
                reversal_jerk: 100_000.0,
            },
        };
        Self {
            schema_version: SCHEMA_VERSION,
            profile_id: "alia250-legacy-v1".to_owned(),
            mode: FeelMode::LegacyCompatibility,
            bindings: ProfileBindings {
                device_profile_sha256: crate::DeviceProfileDigest::from_bytes(
                    LEGACY_DEVICE_PROFILE_SHA256,
                ),
                flight_controller_sha256: crate::FlightControllerDigest::from_bytes(
                    LEGACY_FLIGHT_CONTROLLER_SHA256,
                ),
            },
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

/// The numbers one operator mode chooses, and the shape they are put into.
///
/// The shape is shared so a mode cannot differ in kind from another: every
/// mode releases at least as promptly as it applies and reverses no quicker
/// than it releases, and every band is hysteretic with a dwell.
///
/// The release ordering is the safety-relevant one and it reads backwards to
/// intuition — see [`Self::RELEASE_FACTOR`] for why letting go must never lag
/// asking.
#[derive(Debug, Clone, Copy)]
struct ModeTuning {
    deadzone: f32,
    center_expo: f32,
    outer_expo: f32,
    enter: f32,
    exit: f32,
    dwell_ms: u32,
    apply_accel: f32,
    apply_jerk: f32,
}

impl ModeTuning {
    /// Releasing is PROMPTER than applying, never gentler.
    ///
    /// Comfort would argue the other way — an input returning to centre is a
    /// deceleration nobody asked to be abrupt — and for a car's torque that is
    /// the right argument. It is the wrong one here: letting go is how an
    /// operator stops asking, so a release that lagged the apply would mean
    /// the vehicle took longer to stop commanding than it took to start. The
    /// validator refuses that order, and it is right to.
    ///
    /// What makes a release comfortable is that it is bounded and jerk-limited
    /// at all, not that it is slow. The command law this replaces released at
    /// ten thousand per second with no jerk limit, which is a step.
    const RELEASE_FACTOR: f32 = 1.25;
    /// A reversal is a correction, so it is no slower than a fresh command and
    /// no quicker than a release. It crosses zero under load, which is where an
    /// unshaped law feels like a jolt, and bounding its jerk is what removes
    /// the jolt without making the correction sluggish.
    const REVERSAL_FACTOR: f32 = 1.0;

    fn slug(self, mode: FeelMode) -> &'static str {
        match mode {
            FeelMode::Precision => "precision",
            FeelMode::Balanced => "balanced",
            FeelMode::Agile => "agile",
            FeelMode::LegacyCompatibility => "legacy",
        }
    }

    /// The direct attitude and thrust family at this mode's authority.
    ///
    /// The rates are in radians and normalized thrust rather than in demand
    /// units, so they are derived from the mode's acceleration rather than
    /// shared with it: what carries across is the ordering between modes, not
    /// the number.
    fn direct(self) -> DirectDynamics {
        let scale = f64::from(self.apply_accel) / 4.0;
        DirectDynamics {
            tilt_rate_rps: (1.2 * scale) as f32,
            tilt_accel_rps2: (6.0 * scale) as f32,
            thrust_rate_per_s: (0.9 * scale) as f32,
            thrust_accel_per_s2: (4.5 * scale) as f32,
        }
    }

    /// The brake-to-hold transition at this mode's patience.
    ///
    /// A calmer mode waits longer before it calls the vehicle stopped, which
    /// is the same judgement its longer neutral dwell makes about the stick.
    fn hold(self) -> HoldTransition {
        HoldTransition {
            max_speed_mps: 0.3,
            max_accel_mps2: 0.6,
            require_accel: false,
            stable_dwell_ms: self.dwell_ms,
        }
    }

    /// One axis at a fraction of this mode's authority.
    fn axis(self, scale: f32) -> AxisResponse {
        let accel = self.apply_accel * scale;
        let jerk = self.apply_jerk * scale;
        AxisResponse {
            curve: AxisCurve {
                deadzone: self.deadzone,
                center_expo: self.center_expo,
                outer_expo: self.outer_expo,
                // Where the outer curve starts to blend in. At 1.0 it never
                // does, and a mode's outer exponent is dead configuration
                // that reads as a difference between modes and is not one.
                // Blending over the last third gives the finer centre the
                // exponents are chosen for and still reaches full authority.
                outer_start: 0.7,
            },
            neutral: NeutralBand {
                active_enter: self.enter,
                active_exit: self.exit,
                dwell_ms: self.dwell_ms,
            },
            dynamics: AxisDynamics {
                apply_accel: accel,
                apply_jerk: jerk,
                release_accel: accel * Self::RELEASE_FACTOR,
                release_jerk: jerk * Self::RELEASE_FACTOR,
                reversal_accel: accel * Self::REVERSAL_FACTOR,
                reversal_jerk: jerk * Self::REVERSAL_FACTOR,
            },
        }
    }
}
