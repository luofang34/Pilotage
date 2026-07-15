//! Bounded capture-time interpolation of the coherent aircraft state.
//!
//! The safety-load-bearing quantity is attitude: the horizon and flight-path
//! marker are registered against where the world's down vector and the velocity
//! vector fall in the camera image, so attitude is interpolated with
//! **normalized quaternion interpolation** (nlerp) in the short hemisphere. The
//! blend reads `q` and `-q` as the same rotation — it flips the far sample into
//! the near hemisphere before blending — so a producer's quaternion-sign choice
//! never swings the interpolated attitude the long way round. Velocity and body
//! rate are linear-blended.
//!
//! The geodetic position *value* is deliberately not re-derived here: no
//! conformal cue consumes it (the horizon is at infinity, the flight-path marker
//! is a direction), so it enters only through its accuracy (the parallax term of
//! the error budget) and its coherent identity. Re-deriving a value nothing reads
//! would add antimeridian and datum-realization edge cases for no consumer.

use pilotage_frames::Quat;

use super::KinematicSample;

/// The aircraft state resolved at capture time, plus the timing facts the error
/// budget and the state machine need.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Interpolated {
    /// Body → local-navigation (NED) attitude at capture time.
    pub attitude: Quat,
    /// NED velocity at capture time, meters/second (the flight-path-marker
    /// velocity reference).
    pub velocity_ned_mps: [f64; 3],
    /// Body-frame angular rate at capture time, radians/second.
    pub body_rate_rps: [f32; 3],
    /// The conservative (worst-of-bracket) attitude 1-sigma accuracy, radians.
    pub attitude_sigma_rad: f64,
    /// The conservative (worst-of-bracket) horizontal position 1-sigma accuracy,
    /// meters.
    pub position_sigma_m: f64,
    /// The conservative (worst-of-bracket) velocity 1-sigma accuracy,
    /// meters/second — the velocity-error input to the alignment budget.
    pub velocity_sigma_mps: f64,
    /// Timing facts of the interpolation.
    pub timing: Timing,
}

/// The timing facts of one interpolation: how far the capture time sits outside
/// the bracket (extrapolation), how badly each sample's components are co-timed
/// (skew), and the bracket span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Timing {
    /// Component co-timing skew — the widest epoch spread across a sample's
    /// pose and kinematic epochs, the worst of the two samples, nanoseconds.
    pub skew_ns: u64,
    /// How far the capture time falls outside `[older, newer]`, nanoseconds; `0`
    /// when the capture time is bracketed (a true interpolation).
    pub extrapolation_ns: u64,
    /// The bracket span (newer − older), nanoseconds.
    pub span_ns: u64,
}

/// Blends two unit quaternions on the short hemisphere and renormalizes, so the
/// result is a unit rotation and `q`/`-q` inputs are equivalent. The antipodal
/// degenerate case (a zero-norm blend) fails safe to `a`.
fn nlerp(a: Quat, b: Quat, t: f32) -> Quat {
    let dot = a.w * b.w + a.x * b.x + a.y * b.y + a.z * b.z;
    let s = if dot < 0.0 { -1.0 } else { 1.0 };
    let u = 1.0 - t;
    let (w, x, y, z) = (
        u * a.w + t * s * b.w,
        u * a.x + t * s * b.x,
        u * a.y + t * s * b.y,
        u * a.z + t * s * b.z,
    );
    let n = libm::sqrtf(w * w + x * x + y * y + z * z);
    if n > f32::EPSILON {
        Quat {
            w: w / n,
            x: x / n,
            y: y / n,
            z: z / n,
        }
    } else {
        a
    }
}

fn lerp3_f64(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    let u = 1.0 - t;
    [
        u * a[0] + t * b[0],
        u * a[1] + t * b[1],
        u * a[2] + t * b[2],
    ]
}

fn lerp3_f32(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let u = 1.0 - t;
    [
        u * a[0] + t * b[0],
        u * a[1] + t * b[1],
        u * a[2] + t * b[2],
    ]
}

/// The co-timing spread of one sample: the widest gap among its component
/// epochs — the pose's acquisition epochs plus each kinematic value's tagged
/// epoch and acquisition epoch. A velocity or body rate timed away from the
/// pose widens the spread exactly like an attitude/position skew, so it is
/// charged to the policy skew limit and the latency budget term rather than
/// silently bridged.
fn skew_of(sample: &KinematicSample) -> u64 {
    let epochs = [
        sample.attitude.stamp.acquired_at.nanos,
        sample.position.stamp.acquired_at.nanos,
        sample.velocity.epoch.nanos,
        sample.velocity.meta.acquired_at.nanos,
        sample.body_rate.epoch.nanos,
        sample.body_rate.meta.acquired_at.nanos,
    ];
    let hi = epochs.into_iter().max().unwrap_or(0);
    let lo = epochs.into_iter().min().unwrap_or(0);
    hi.saturating_sub(lo)
}

/// Interpolates the coherent bracket at `capture_ns`. The caller has already
/// validated that both samples and the capture time share one clock and scale,
/// that the samples are one continuous stream, and that `older` is not after
/// `newer`; this function is the pure numeric blend over that validated bracket.
pub(crate) fn interpolate(
    older: &KinematicSample,
    newer: &KinematicSample,
    capture_ns: u64,
) -> Interpolated {
    let older_ns = older.attitude.stamp.acquired_at.nanos;
    let newer_ns = newer.attitude.stamp.acquired_at.nanos;
    let span_ns = newer_ns.saturating_sub(older_ns);

    // The blend fraction is clamped into `[0, 1]`: past either end the nearest
    // endpoint is used and the extrapolation distance is charged to the budget
    // rather than propagated forward into a fabricated pose.
    let raw = if span_ns == 0 {
        0.0
    } else {
        (capture_ns as f64 - older_ns as f64) / span_ns as f64
    };
    let t = raw.clamp(0.0, 1.0);

    // Distance the capture time falls outside `[older, newer]`; one of the two
    // saturating differences is zero whenever the capture time is bracketed.
    let extrapolation_ns = older_ns
        .saturating_sub(capture_ns)
        .max(capture_ns.saturating_sub(newer_ns));

    let attitude_sigma_rad = sigma_att(older).max(sigma_att(newer));
    let position_sigma_m = sigma_pos(older).max(sigma_pos(newer));
    let velocity_sigma_mps = sigma_vel(older).max(sigma_vel(newer));

    Interpolated {
        attitude: nlerp(older.attitude.attitude, newer.attitude.attitude, t as f32),
        velocity_ned_mps: lerp3_f64(older.velocity.value, newer.velocity.value, t),
        body_rate_rps: lerp3_f32(older.body_rate.value, newer.body_rate.value, t as f32),
        attitude_sigma_rad,
        position_sigma_m,
        velocity_sigma_mps,
        timing: Timing {
            skew_ns: skew_of(older).max(skew_of(newer)),
            extrapolation_ns,
            span_ns,
        },
    }
}

fn sigma_att(sample: &KinematicSample) -> f64 {
    f64::from(sample.attitude.quality.angular_mrad) * 1e-3
}

fn sigma_pos(sample: &KinematicSample) -> f64 {
    f64::from(sample.position.quality.horizontal_mm) * 1e-3
}

fn sigma_vel(sample: &KinematicSample) -> f64 {
    f64::from(sample.velocity_quality.sigma_mmps) * 1e-3
}
