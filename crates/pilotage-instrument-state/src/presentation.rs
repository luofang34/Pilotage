//! SO(3)-safe attitude presentation: geometry from the physical down
//! vector, and the unusual-attitude tier machine (ATT-01).
//!
//! The canonical display attitude is the validated unit quaternion; the
//! horizon geometry derives from where the world's down vector points in
//! the body frame, using only quadratic quaternion forms so `q` and `-q`
//! produce bit-identical output. Display pitch is the nose's angle to
//! the horizontal; display bank rotates the horizon and stays continuous
//! through the vertical, where it is *held* (the parameter chart is
//! singular there, the rendered geometry is not). Sky stays above the
//! horizon line whenever `|bank| < 90°`; inverted flight reads as bank
//! beyond 90°, never as a relabeled sky/ground.
//!
//! Tier decisions (unusual entry/exit, chevrons, declutter) come from an
//! [`AirframeDisplayProfile`] with explicit entry/exit hysteresis. The
//! simulator profile carries fixed-wing benchmark numbers as data; it
//! implies no aircraft approval.

use libm::{asinf, atan2f};

use crate::quat::Quat;

/// Why an airframe display profile could not be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    /// A threshold is NaN or infinite.
    NonFinite,
    /// An exit threshold does not sit strictly inside its entry
    /// threshold, so the tier could chatter.
    NoHysteresis,
    /// A threshold is outside its physical range.
    OutOfRange,
}

/// Entry/exit angle thresholds (radians) for one hysteresis pair.
///
/// Entry fires at or beyond `entry`; exit requires coming back inside
/// `exit`. `exit` must sit strictly inside `entry`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hysteresis {
    /// Magnitude at which the condition engages.
    pub entry: f32,
    /// Magnitude below which the condition releases.
    pub exit: f32,
}

impl Hysteresis {
    fn valid(&self, max: f32) -> bool {
        self.entry.is_finite()
            && self.exit.is_finite()
            && self.exit > 0.0
            && self.exit < self.entry
            && self.entry <= max
    }
}

/// Display thresholds for one airframe (ATT-01).
///
/// All values are profile *data*: the simulator profile's numbers are
/// fixed-wing benchmark inputs (G5000-class), not an approval for any
/// aircraft. Constructed only through [`AirframeDisplayProfile::new`] or
/// [`AirframeDisplayProfile::simulator`], so inverted or non-finite
/// thresholds cannot exist.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AirframeDisplayProfile {
    unusual_pitch_up: Hysteresis,
    unusual_pitch_down: Hysteresis,
    unusual_bank: Hysteresis,
    chevron_pitch_up: Hysteresis,
    chevron_pitch_down: Hysteresis,
    /// |pitch| beyond which display bank holds its last value (the
    /// vertical parameter singularity).
    bank_hold_pitch: Hysteresis,
}

/// The raw threshold set [`AirframeDisplayProfile::new`] validates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileLimits {
    /// Nose-high unusual-attitude entry/exit.
    pub unusual_pitch_up: Hysteresis,
    /// Nose-low unusual-attitude entry/exit (magnitudes).
    pub unusual_pitch_down: Hysteresis,
    /// Bank unusual-attitude entry/exit (magnitudes).
    pub unusual_bank: Hysteresis,
    /// Nose-high recovery-chevron entry/exit.
    pub chevron_pitch_up: Hysteresis,
    /// Nose-low recovery-chevron entry/exit (magnitudes).
    pub chevron_pitch_down: Hysteresis,
    /// Bank-hold pitch magnitude entry/exit.
    pub bank_hold_pitch: Hysteresis,
}

const DEG: f32 = core::f32::consts::PI / 180.0;
const HALF_PI: f32 = core::f32::consts::FRAC_PI_2;
const PI: f32 = core::f32::consts::PI;

impl AirframeDisplayProfile {
    /// Builds a profile after validating every threshold pair.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] when a threshold is non-finite, outside
    /// its physical range, or lacks hysteresis.
    pub fn new(limits: ProfileLimits) -> Result<Self, ProfileError> {
        let pairs = [
            (limits.unusual_pitch_up, HALF_PI),
            (limits.unusual_pitch_down, HALF_PI),
            (limits.unusual_bank, PI),
            (limits.chevron_pitch_up, HALF_PI),
            (limits.chevron_pitch_down, HALF_PI),
            (limits.bank_hold_pitch, HALF_PI),
        ];
        let mut i = 0;
        while i < pairs.len() {
            let (pair, max) = pairs[i];
            if !(pair.entry.is_finite() && pair.exit.is_finite()) {
                return Err(ProfileError::NonFinite);
            }
            if !pair.valid(max) {
                return Err(ProfileError::NoHysteresis);
            }
            i += 1;
        }
        Ok(Self {
            unusual_pitch_up: limits.unusual_pitch_up,
            unusual_pitch_down: limits.unusual_pitch_down,
            unusual_bank: limits.unusual_bank,
            chevron_pitch_up: limits.chevron_pitch_up,
            chevron_pitch_down: limits.chevron_pitch_down,
            bank_hold_pitch: limits.bank_hold_pitch,
        })
    }

    /// The simulator profile. Its numbers are fixed-wing *benchmark
    /// inputs* (declutter at +30/-20° pitch or 65° bank; chevrons at
    /// +50/-30° pitch, each with a 5° exit margin; bank hold beyond 88°
    /// pitch). They are profile data for the simulator display only and
    /// imply no aircraft approval.
    pub fn simulator() -> Self {
        Self {
            unusual_pitch_up: Hysteresis {
                entry: 30.0 * DEG,
                exit: 25.0 * DEG,
            },
            unusual_pitch_down: Hysteresis {
                entry: 20.0 * DEG,
                exit: 15.0 * DEG,
            },
            unusual_bank: Hysteresis {
                entry: 65.0 * DEG,
                exit: 60.0 * DEG,
            },
            chevron_pitch_up: Hysteresis {
                entry: 50.0 * DEG,
                exit: 45.0 * DEG,
            },
            chevron_pitch_down: Hysteresis {
                entry: 30.0 * DEG,
                exit: 25.0 * DEG,
            },
            bank_hold_pitch: Hysteresis {
                entry: 88.0 * DEG,
                exit: 87.0 * DEG,
            },
        }
    }
}

/// Which way recovery chevrons point: always toward the horizon, never
/// a flight-director command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChevronSense {
    /// Nose high: the horizon is below; chevrons point down.
    HorizonBelow,
    /// Nose low: the horizon is above; chevrons point up.
    HorizonAbove,
}

/// The attitude presentation one frame renders from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttitudePresentation {
    /// Display bank, radians, continuous through the vertical (held at
    /// the singularity); positive right wing down. `|bank| > 90°` is
    /// inverted flight.
    pub bank_rad: f32,
    /// Display pitch, radians, the nose's angle above the horizontal in
    /// `[-90°, +90°]`.
    pub pitch_rad: f32,
    /// Unusual-attitude tier is engaged (drives declutter).
    pub unusual: bool,
    /// The nose-high condition is latched.
    pub nose_high: bool,
    /// The nose-low condition is latched.
    pub nose_low: bool,
    /// The high-bank condition is latched.
    pub high_bank: bool,
    /// The display reads inverted (`|bank| > 90°`).
    pub inverted: bool,
    /// Recovery chevrons to draw, pointing toward the horizon.
    pub chevrons: Option<ChevronSense>,
}

impl Default for AttitudePresentation {
    fn default() -> Self {
        Self {
            bank_rad: 0.0,
            pitch_rad: 0.0,
            unusual: false,
            nose_high: false,
            nose_low: false,
            high_bank: false,
            inverted: false,
            chevrons: None,
        }
    }
}

/// Hysteresis latches carried across frames. Allocation-free; the
/// display backend owns one per attitude source and resets it whenever
/// the attitude is invalid, so recovery re-enters from a clean tier.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UnusualAttitudeState {
    nose_high: bool,
    nose_low: bool,
    high_bank: bool,
    chevron_up: bool,
    chevron_down: bool,
    bank_hold: bool,
    held_bank_rad: f32,
    has_held_bank: bool,
}

impl UnusualAttitudeState {
    /// Clears every latch (invalid attitude, source change, reinit).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Advances the tier machine with a validated unit quaternion and
    /// returns the frame's presentation. Deterministic: same state and
    /// input produce the same output, and `q`/`-q` are identical because
    /// only quadratic forms are read.
    pub fn step(&mut self, quat: Quat, profile: &AirframeDisplayProfile) -> AttitudePresentation {
        let (down_x, down_y, down_z) = down_in_body(quat);
        let pitch = -asinf(down_x.clamp(-1.0, 1.0));

        // Through the vertical the bank parameter is singular; hold the
        // last well-defined value so the ball cannot spin (the rendered
        // sky vector stays continuous either way).
        let vertical = latch(
            &mut self.bank_hold,
            pitch.abs(),
            profile.bank_hold_pitch.entry,
            profile.bank_hold_pitch.exit,
        );
        let bank = if vertical && self.has_held_bank {
            self.held_bank_rad
        } else {
            let raw = atan2f(down_y, down_z);
            self.held_bank_rad = raw;
            self.has_held_bank = true;
            raw
        };

        let nose_high = latch(
            &mut self.nose_high,
            pitch,
            profile.unusual_pitch_up.entry,
            profile.unusual_pitch_up.exit,
        );
        let nose_low = latch(
            &mut self.nose_low,
            -pitch,
            profile.unusual_pitch_down.entry,
            profile.unusual_pitch_down.exit,
        );
        let high_bank = latch(
            &mut self.high_bank,
            bank.abs(),
            profile.unusual_bank.entry,
            profile.unusual_bank.exit,
        );
        let chevron_up = latch(
            &mut self.chevron_up,
            pitch,
            profile.chevron_pitch_up.entry,
            profile.chevron_pitch_up.exit,
        );
        let chevron_down = latch(
            &mut self.chevron_down,
            -pitch,
            profile.chevron_pitch_down.entry,
            profile.chevron_pitch_down.exit,
        );

        AttitudePresentation {
            bank_rad: bank,
            pitch_rad: pitch,
            unusual: nose_high || nose_low || high_bank,
            nose_high,
            nose_low,
            high_bank,
            inverted: bank.abs() > HALF_PI,
            chevrons: if chevron_up {
                Some(ChevronSense::HorizonBelow)
            } else if chevron_down {
                Some(ChevronSense::HorizonAbove)
            } else {
                None
            },
        }
    }
}

/// One hysteresis latch: engages at or beyond `entry`, releases only
/// inside `exit`.
fn latch(state: &mut bool, magnitude: f32, entry: f32, exit: f32) -> bool {
    if *state {
        if magnitude < exit {
            *state = false;
        }
    } else if magnitude >= entry {
        *state = true;
    }
    *state
}

/// The world down vector expressed in body coordinates, from quadratic
/// quaternion forms only (the third row of the body→NED rotation), so
/// `q` and `-q` are bit-identical. Level flight reads `(0, 0, 1)`.
pub fn down_in_body(quat: Quat) -> (f32, f32, f32) {
    let Quat { w, x, y, z } = quat;
    (
        2.0 * (x * z - w * y),
        2.0 * (y * z + w * x),
        1.0 - 2.0 * (x * x + y * y),
    )
}

#[cfg(test)]
mod tests;
