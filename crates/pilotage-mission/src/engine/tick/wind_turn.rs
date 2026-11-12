//! A fixed wing's turn in a steady wind: at a constant bank and airspeed
//! it turns at a constant rate through the air, and the wind carries the
//! whole turn downwind, so over the ground it is no circle. The turn ahead
//! of a fly-by waypoint starts where that path, which ends on the next
//! leg's course, ends on the next leg.

use core::f64::consts::PI;

/// The heading step the turn is followed in, radians (half a degree).
const STEP_RAD: f64 = PI / 360.0;

/// The most steps followed: more than a half turn's worth.
const MAX_STEPS: u32 = 1_000;

/// How far before the waypoint, metres, a turn from the ground course
/// `into` to `out` (radians) starts, flown at `airspeed_mps` and a bank of
/// `bank_rad` in `wind` (north and east, m/s), for it to end on the next
/// leg. `None` for no turn, legs too close to parallel, a wind the
/// airspeed cannot make good across either course, or a turn that does
/// not end.
pub(super) fn lead_m(
    into: f64,
    out: f64,
    airspeed_mps: f64,
    wind: [f64; 2],
    bank_rad: f64,
) -> Option<f64> {
    let turn = wrap(out - into);
    let sense = turn.signum();
    let rate = crate::body::SURFACE_GRAVITY_MPS2 * bank_rad.tan() / airspeed_mps;
    if turn.abs() < STEP_RAD || !(rate > 0.0 && rate.is_finite()) {
        return None;
    }
    let start = heading_for(into, airspeed_mps, wind)?;
    heading_for(out, airspeed_mps, wind)?;
    let dt = STEP_RAD / rate;
    let mut moved = [0.0_f64; 2];
    let ground_on = |heading: f64| {
        [
            airspeed_mps * heading.cos() + wind[0],
            airspeed_mps * heading.sin() + wind[1],
        ]
    };
    let to_go = |ground: [f64; 2]| sense * wrap(out - ground[1].atan2(ground[0]));
    for step in 0..MAX_STEPS {
        let heading = start + sense * STEP_RAD * f64::from(step);
        let ground = ground_on(heading);
        let (before, after) = (to_go(ground), to_go(ground_on(heading + sense * STEP_RAD)));
        // The last step is taken only as far as the next leg's course.
        let share = if after <= 0.0 {
            before / (before - after)
        } else {
            1.0
        };
        // The step moves at the ground velocity of its middle heading.
        let middle = ground_on(heading + sense * STEP_RAD * share / 2.0);
        moved = [
            moved[0] + middle[0] * dt * share,
            moved[1] + middle[1] * dt * share,
        ];
        if after <= 0.0 {
            return lead_to_end(into, out, moved);
        }
    }
    None
}

/// How far before the waypoint a turn starts that moves `moved` (north and
/// east, metres) from a start on the inbound leg on `into` to an end on the
/// next leg on `out`: `d u_in + moved` lies along `u_out`, so its cross
/// product with `u_out` is zero.
fn lead_to_end(into: f64, out: f64, moved: [f64; 2]) -> Option<f64> {
    let (u_in, u_out) = (unit(into), unit(out));
    let across = cross(u_out, u_in);
    if across.abs() < 1e-6 {
        return None;
    }
    let lead = cross(u_out, moved) / across;
    (lead.is_finite() && lead >= 0.0).then_some(lead)
}

/// The heading that flies the ground course `course` at `airspeed_mps` in
/// `wind`: turned into the wind by the crosswind over the airspeed.
fn heading_for(course: f64, airspeed_mps: f64, wind: [f64; 2]) -> Option<f64> {
    let across = -wind[0] * course.sin() + wind[1] * course.cos();
    let ratio = across / airspeed_mps;
    (ratio.abs() < 1.0).then(|| course - ratio.asin())
}

fn unit(course: f64) -> [f64; 2] {
    [course.cos(), course.sin()]
}

fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn wrap(angle: f64) -> f64 {
    angle.sin().atan2(angle.cos())
}

#[cfg(test)]
mod tests;
