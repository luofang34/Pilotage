//! What an airframe can actually do, as far as the operator's demand goes.
//!
//! A demand law shapes what full stick MEANS. It can ask for anything; the
//! vehicle answers with what its velocity loop can produce. Where the ask
//! exceeds the answer the shaping stops being felt at all — the operator feels
//! the airframe's limit instead, and two modes that differ only above that
//! limit feel identical. So the ceiling belongs beside the law, stated in the
//! airframe's own terms rather than assumed.

/// Standard gravity, m/s².
const GRAVITY_MPS2: f32 = 9.806_65;

/// The airframe facts a demand law has to respect.
///
/// These mirror one airframe preset each. They are recorded here rather than
/// read at runtime because a shipped profile is an artifact: it has to be the
/// same law every time it is loaded, and a law that changed with a file in
/// another repository would not be.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AirframeLimits {
    /// The name shipped profiles for this airframe are filed under.
    pub id: &'static str,
    /// Greatest roll/pitch tilt the velocity loop may command, radians.
    ///
    /// `gains.vel_max_roll_pitch` in the airframe preset.
    pub max_tilt_rad: f32,
}

impl AirframeLimits {
    /// The gz X500 quadrotor.
    pub const X500: Self = Self {
        id: "x500",
        max_tilt_rad: 0.35,
    };

    /// The Alia 250, which may tilt further and so may accelerate harder.
    pub const ALIA250: Self = Self {
        id: "alia250",
        max_tilt_rad: 0.45,
    };

    /// The greatest horizontal acceleration this airframe can produce.
    ///
    /// Level flight at a tilt of `θ` accelerates at `g·tan(θ)`: the rotors
    /// hold the weight and the horizontal component is what is left over.
    /// Demanding a slew above this asks for a velocity the vehicle cannot
    /// reach on time, and the demand runs away from what is under it.
    #[must_use]
    pub fn horizontal_accel_ceiling_mps2(&self) -> f32 {
        GRAVITY_MPS2 * self.max_tilt_rad.tan()
    }

    /// How much of one mode's ask this airframe can actually deliver.
    ///
    /// At or below `1.0` the operator feels the law. Above it they feel the
    /// airframe, and the part of the mode above the ceiling is a number in a
    /// file rather than a difference anyone can fly.
    #[must_use]
    pub fn share_of_ceiling(&self, apply_accel_mps2: f32) -> f32 {
        apply_accel_mps2 / self.horizontal_accel_ceiling_mps2()
    }
}

#[cfg(test)]
mod tests;
