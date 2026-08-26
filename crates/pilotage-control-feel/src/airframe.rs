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

/// How much of the airframe's authority the most aggressive mode may ask for.
///
/// Not all of it. The velocity loop has to track the demand AND hold against
/// wind and estimator error at the same time; a demand sitting exactly at the
/// tilt limit leaves nothing to do the second job with, and the first thing a
/// gust costs is the ability to follow the stick.
const MOST_AGGRESSIVE_SHARE: f32 = 0.9;

impl crate::FlightFeelProfile {
    /// The shaped law for one mode, fitted to what this airframe can deliver.
    ///
    /// Modes that already sit inside the ceiling are returned as they are. A
    /// mode above it is brought down to the ceiling, and its siblings are left
    /// alone: a law that has been flown is evidence, and moving it to make room
    /// for a mode that was never flyable spends that evidence for nothing.
    ///
    /// The exception is an airframe tight enough that clipping lands two modes
    /// on the same number — a control offering two names for one law. There the
    /// whole family scales instead, which costs every mode some authority but
    /// keeps the felt distance between them.
    ///
    /// Which branch an airframe takes is a fact about that airframe, not a
    /// preference: see the shipped-law tests for what each one does and why.
    #[must_use]
    pub fn shaped_for(limits: AirframeLimits, mode: crate::FeelMode) -> Self {
        // The compatibility law is exempt, and must be: the adapter refuses a
        // legacy profile that is not byte-identical to the one it compiles in,
        // so shaping it would make the file unloadable rather than flyable.
        //
        // It is also the only law a physical vehicle is allowed to fly, and it
        // sits outside BOTH airframes. Nothing here fixes that; see
        // `the_law_a_physical_vehicle_flies_is_outside_both_airframes`.
        if mode == crate::FeelMode::LegacyCompatibility {
            return Self::shaped(mode);
        }
        let allowed = limits.horizontal_accel_ceiling_mps2() * MOST_AGGRESSIVE_SHARE;
        let asks = |mode| Self::shaped(mode).horizontal.dynamics.apply_accel;
        let (precision, balanced, agile) = (
            asks(crate::FeelMode::Precision),
            asks(crate::FeelMode::Balanced),
            asks(crate::FeelMode::Agile),
        );

        // Clip first, and keep the modes a reader has already flown wherever
        // clipping leaves them distinct. Scaling the whole family costs every
        // mode authority to bring ONE of them inside the ceiling, which is a
        // change to laws that were not the problem.
        let clipped = [
            precision.min(allowed),
            balanced.min(allowed),
            agile.min(allowed),
        ];
        // Unless clipping collapses two modes onto one number. Then the control
        // offers two names for one law, which is the defect this exists to
        // prevent, and scaling the family is the lesser cost.
        let collapses = clipped[0] >= clipped[1] || clipped[1] >= clipped[2];
        let scale = if collapses { allowed / agile } else { 1.0 };

        let mut profile = Self::shaped(mode);
        let ask = profile.horizontal.dynamics.apply_accel;
        // How much this mode's numbers move: the clip, or the family scale.
        let factor = if collapses {
            scale
        } else {
            ask.min(allowed) / ask
        };
        if factor < 1.0 {
            for axis in [
                &mut profile.horizontal,
                &mut profile.vertical,
                &mut profile.yaw,
            ] {
                axis.dynamics.apply_accel *= factor;
                axis.dynamics.apply_jerk *= factor;
                axis.dynamics.release_accel *= factor;
                axis.dynamics.release_jerk *= factor;
                axis.dynamics.reversal_accel *= factor;
                axis.dynamics.reversal_jerk *= factor;
            }
        }
        profile.profile_id = format!(
            "{}-shaped-{}-v1",
            limits.id,
            profile
                .profile_id
                .rsplit_once("-v1")
                .and_then(|(head, _)| head.rsplit_once('-'))
                .map_or("unknown", |(_, slug)| slug)
        );
        profile
    }
}

#[cfg(test)]
mod tests;
