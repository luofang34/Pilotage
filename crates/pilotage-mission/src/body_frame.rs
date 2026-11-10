//! Horizontal NED to body-FRD rotation, yaw geometry, and pure `f64`
//! projection of outer-loop guidance setpoints into typed control intents.

use core::f64::consts::{FRAC_PI_2, PI};

use navigate_contract::{GuidanceSetpoint, NedVelocity};
use pilotage_protocol::{AttitudeThrustIntent, ControlIntent, ReferenceFrame, VelocityIntent};

use crate::config::MissionLimits;

/// Commanded horizontal speed below which course has no useful direction.
pub(crate) const YAW_COURSE_FLOOR_MPS: f64 = 0.05;

/// Rotates a horizontal NED velocity into body FRD at yaw `psi`
/// (radians, zero north, positive east):
/// `vx = vn·cosψ + ve·sinψ`, `vy = −vn·sinψ + ve·cosψ`.
pub(crate) fn ned_to_body(vn: f64, ve: f64, psi: f64) -> (f64, f64) {
    let (sin, cos) = psi.sin_cos();
    (vn * cos + ve * sin, -vn * sin + ve * cos)
}

/// Bearing of a horizontal NED vector, radians in `(-π, π]`, zero
/// north, positive toward east.
pub(crate) fn bearing_rad(vn: f64, ve: f64) -> f64 {
    ve.atan2(vn)
}

/// Wraps an angle to `(-π, π]` so a heading error never asks for the
/// long way around.
pub(crate) fn wrap_to_pi(angle: f64) -> f64 {
    let wrapped = (-angle + PI).rem_euclid(2.0 * PI);
    -(wrapped - PI)
}

/// Clamps to `[-limit, limit]`.
pub(crate) fn clamp_symmetric(value: f64, limit: f64) -> f64 {
    value.clamp(-limit, limit)
}

/// Caps a horizontal vector's magnitude by scaling, so the ceiling
/// trades speed for nothing else — the direction survives.
pub(crate) fn cap_horizontal(vn: f64, ve: f64, max: f64) -> (f64, f64) {
    let speed = (vn * vn + ve * ve).sqrt();
    if speed <= max || speed <= 0.0 {
        (vn, ve)
    } else {
        let scale = max / speed;
        (vn * scale, ve * scale)
    }
}

/// Projects an outer-loop [`NedVelocity`] into a body-frame [`ControlIntent::Velocity`]
/// bounded by `limits` and aligned toward the horizontal course at `yaw_gain_per_s`.
#[must_use]
pub fn project_ned_velocity(
    velocity: &NedVelocity,
    yaw_rad: f64,
    limits: &MissionLimits,
    yaw_gain_per_s: f64,
) -> ControlIntent {
    let (vn, ve) = cap_horizontal(
        velocity.north_mps,
        velocity.east_mps,
        limits.max_horizontal_mps,
    );
    let vd = clamp_symmetric(velocity.down_mps, limits.max_vertical_mps);
    let (vx, vy) = ned_to_body(vn, ve, yaw_rad);
    let horizontal_mps = (vn * vn + ve * ve).sqrt();
    let yaw_rate = if horizontal_mps < YAW_COURSE_FLOOR_MPS {
        0.0
    } else {
        let course = bearing_rad(vn, ve);
        clamp_symmetric(
            yaw_gain_per_s * wrap_to_pi(course - yaw_rad),
            limits.max_yaw_rate_rps,
        )
    };
    ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx: vx as f32,
        vy: vy as f32,
        vz: vd as f32,
        yaw_rate: yaw_rate as f32,
    })
}

/// Projects a [`GuidanceSetpoint`] into a typed [`ControlIntent`] under `limits`.
///
/// Supports [`GuidanceSetpoint::Velocity`] and [`GuidanceSetpoint::DeviationTracking`],
/// returning `None` for setpoints that cannot be projected without an external position frame.
#[must_use]
pub fn project_guidance_setpoint(
    setpoint: &GuidanceSetpoint,
    yaw_rad: f64,
    limits: &MissionLimits,
    yaw_gain_per_s: f64,
) -> Option<ControlIntent> {
    match setpoint {
        GuidanceSetpoint::Velocity { velocity } => Some(project_ned_velocity(
            velocity,
            yaw_rad,
            limits,
            yaw_gain_per_s,
        )),
        GuidanceSetpoint::DeviationTracking {
            lateral_m,
            vertical_m,
            course_rad,
        } => {
            let (sin_c, cos_c) = course_rad.sin_cos();
            let (sin_r, cos_r) = (course_rad + FRAC_PI_2).sin_cos();
            let along_mps = 0.8 * limits.max_horizontal_mps;
            let cross_mps = clamp_symmetric(-0.5 * *lateral_m, limits.max_horizontal_mps);
            let vn = along_mps * cos_c + cross_mps * cos_r;
            let ve = along_mps * sin_c + cross_mps * sin_r;
            let vd = clamp_symmetric(0.5 * *vertical_m, limits.max_vertical_mps);
            let velocity = NedVelocity::new(vn, ve, vd);
            Some(project_ned_velocity(
                &velocity,
                yaw_rad,
                limits,
                yaw_gain_per_s,
            ))
        }
        _ => None,
    }
}

/// Projects a 3-axis Cartesian velocity setpoint in `LocalNed` (`[vn_mps, ve_mps, vd_mps]`)
/// into a bounded [`ControlIntent::Velocity`] in `BodyFrd`.
#[must_use]
pub fn project_cartesian_velocity_ned(
    velocity_ned_mps: [f64; 3],
    yaw_rad: f64,
    limits: &MissionLimits,
    yaw_gain_per_s: f64,
) -> ControlIntent {
    let ned = NedVelocity::new(
        velocity_ned_mps[0],
        velocity_ned_mps[1],
        velocity_ned_mps[2],
    );
    project_ned_velocity(&ned, yaw_rad, limits, yaw_gain_per_s)
}

/// Projects a 3-axis Cartesian velocity error vector already in `BodyFrd`
/// (`[vx_mps, vy_mps, vz_mps]`) for 6-DOF proximity/rendezvous RCS translation
/// into a bounded [`ControlIntent::Velocity`].
#[must_use]
pub fn project_cartesian_velocity_body(
    velocity_body_frd_mps: [f64; 3],
    yaw_rate_rps: f64,
    limits: &MissionLimits,
) -> ControlIntent {
    let (vx, vy) = cap_horizontal(
        velocity_body_frd_mps[0],
        velocity_body_frd_mps[1],
        limits.max_horizontal_mps,
    );
    let vz = clamp_symmetric(velocity_body_frd_mps[2], limits.max_vertical_mps);
    let yaw_rate = clamp_symmetric(yaw_rate_rps, limits.max_yaw_rate_rps);
    ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx: vx as f32,
        vy: vy as f32,
        vz: vz as f32,
        yaw_rate: yaw_rate as f32,
    })
}

/// Projects an outer-loop burn or powered-descent thrust vector in `LocalNed`
/// into a [`ControlIntent::AttitudeThrust`] command.
#[must_use]
pub fn project_thrust_vector_ned(
    direction_unit_ned: [f64; 3],
    specific_force_mps2: f64,
    max_specific_force_mps2: f64,
    cutoff: bool,
) -> ControlIntent {
    let [dn, de, dd] = direction_unit_ned;
    let norm = (dn * dn + de * de + dd * dd).sqrt();
    let (qw, qx, qy, qz) = if norm <= 1.0e-9 {
        (1.0_f64, 0.0_f64, 0.0_f64, 0.0_f64)
    } else {
        let (un, ue, ud) = (dn / norm, de / norm, dd / norm);
        let horiz = (un * un + ue * ue).sqrt();
        let yaw = ue.atan2(un);
        let pitch = (-ud).atan2(horiz);
        euler_zyx_to_quat(0.0, pitch, yaw)
    };
    let thrust = if cutoff || specific_force_mps2 <= 0.0 || max_specific_force_mps2 <= 0.0 {
        0.0_f32
    } else {
        (specific_force_mps2 / max_specific_force_mps2).clamp(0.0, 1.0) as f32
    };
    ControlIntent::AttitudeThrust(AttitudeThrustIntent {
        frame: ReferenceFrame::LocalNed,
        qw: qw as f32,
        qx: qx as f32,
        qy: qy as f32,
        qz: qz as f32,
        thrust,
    })
}

/// Projects an outer-loop atmospheric entry bank-angle modulation setpoint
/// into an unpowered [`ControlIntent::AttitudeThrust`] in `LocalNed`.
#[must_use]
pub fn project_entry_bank_ned(bank_rad: f64, heading_course_rad: f64) -> ControlIntent {
    let (qw, qx, qy, qz) = euler_zyx_to_quat(bank_rad, 0.0, heading_course_rad);
    ControlIntent::AttitudeThrust(AttitudeThrustIntent {
        frame: ReferenceFrame::LocalNed,
        qw: qw as f32,
        qx: qx as f32,
        qy: qy as f32,
        qz: qz as f32,
        thrust: 0.0,
    })
}

fn euler_zyx_to_quat(roll: f64, pitch: f64, yaw: f64) -> (f64, f64, f64, f64) {
    let (sr, cr) = (0.5 * roll).sin_cos();
    let (sp, cp) = (0.5 * pitch).sin_cos();
    let (sy, cy) = (0.5 * yaw).sin_cos();
    let qw = cr * cp * cy + sr * sp * sy;
    let qx = sr * cp * cy - cr * sp * sy;
    let qy = cr * sp * cy + sr * cp * sy;
    let qz = cr * cp * sy - sr * sp * cy;
    (qw, qx, qy, qz)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use core::f64::consts::{FRAC_PI_2, PI};

    use navigate_contract::{GeodeticPosition, GuidanceSetpoint, NedVelocity};
    use pilotage_protocol::{ControlIntent, ReferenceFrame};

    use crate::config::MissionLimits;

    use super::{
        bearing_rad, cap_horizontal, ned_to_body, project_cartesian_velocity_body,
        project_cartesian_velocity_ned, project_entry_bank_ned, project_guidance_setpoint,
        project_thrust_vector_ned, wrap_to_pi,
    };

    const EPS: f64 = 1e-12;

    #[test]
    fn rotation_signs_pinned_by_hand_computed_cases() {
        // Facing north: body equals NED.
        let (vx, vy) = ned_to_body(1.0, 0.5, 0.0);
        assert!((vx - 1.0).abs() < EPS && (vy - 0.5).abs() < EPS);
        // Facing east, moving north: pure leftward body motion.
        let (vx, vy) = ned_to_body(1.0, 0.0, FRAC_PI_2);
        assert!(vx.abs() < EPS && (vy + 1.0).abs() < EPS);
        // Facing east, moving east: pure forward.
        let (vx, vy) = ned_to_body(0.0, 1.0, FRAC_PI_2);
        assert!((vx - 1.0).abs() < EPS && vy.abs() < EPS);
        // Facing northeast, moving north: forward-left at 45 degrees.
        let (vx, vy) = ned_to_body(1.0, 0.0, PI / 4.0);
        let expected = FRAC_PI_2.sin() / 2.0_f64.sqrt();
        assert!((vx - expected).abs() < 1e-9 && (vy + expected).abs() < 1e-9);
    }

    #[test]
    fn bearing_is_zero_north_positive_east() {
        assert!(bearing_rad(1.0, 0.0).abs() < EPS);
        assert!((bearing_rad(0.0, 1.0) - FRAC_PI_2).abs() < EPS);
        assert!((bearing_rad(-1.0, 0.0) - PI).abs() < EPS);
        assert!((bearing_rad(0.0, -1.0) + FRAC_PI_2).abs() < EPS);
    }

    #[test]
    fn wrap_keeps_errors_on_the_short_way() {
        assert!((wrap_to_pi(3.0 * PI / 2.0) + FRAC_PI_2).abs() < EPS);
        assert!((wrap_to_pi(-3.0 * PI / 2.0) - FRAC_PI_2).abs() < EPS);
        assert!((wrap_to_pi(PI) - PI).abs() < EPS);
        assert!(wrap_to_pi(2.0 * PI).abs() < EPS);
    }

    #[test]
    fn horizontal_cap_scales_without_rotating() {
        let (vn, ve) = cap_horizontal(3.0, 4.0, 2.5);
        let speed = (vn * vn + ve * ve).sqrt();
        assert!((speed - 2.5).abs() < 1e-9);
        assert!((vn / ve - 3.0 / 4.0).abs() < 1e-9);
        let (vn, ve) = cap_horizontal(1.0, 1.0, 2.5);
        assert!((vn - 1.0).abs() < EPS && (ve - 1.0).abs() < EPS);
    }

    #[test]
    fn guidance_setpoint_projection_handles_velocity_and_deviation_tracking() {
        let limits = MissionLimits::default();
        let vel_sp = GuidanceSetpoint::Velocity {
            velocity: NedVelocity::new(2.0, 0.0, -0.5),
        };
        let Some(ControlIntent::Velocity(v)) =
            project_guidance_setpoint(&vel_sp, 0.0, &limits, 1.0)
        else {
            panic!("expected velocity intent");
        };
        assert_eq!(v.frame, ReferenceFrame::BodyFrd);
        assert!((v.vx - 2.0).abs() < 1e-5);
        assert!((v.vz + 0.5).abs() < 1e-5);

        let dev_sp = GuidanceSetpoint::DeviationTracking {
            lateral_m: 1.0,
            vertical_m: 0.6,
            course_rad: 0.0,
        };
        let Some(ControlIntent::Velocity(dev_v)) =
            project_guidance_setpoint(&dev_sp, 0.0, &limits, 1.0)
        else {
            panic!("expected deviation tracking velocity intent");
        };
        assert!(dev_v.vx > 0.0);
        assert!(dev_v.vy < 0.0);
        assert!(dev_v.vz > 0.0);

        let pos_sp = GuidanceSetpoint::Position {
            target: GeodeticPosition::new(0.0, 0.0, 0.0),
        };
        assert!(project_guidance_setpoint(&pos_sp, 0.0, &limits, 1.0).is_none());
    }

    #[test]
    fn thrust_vector_and_entry_bank_project_to_unit_quaternion_attitude_thrust() {
        let ControlIntent::AttitudeThrust(active) =
            project_thrust_vector_ned([0.0, 0.0, -1.0], 19.6, 39.2, false)
        else {
            panic!("expected attitude thrust intent");
        };
        assert_eq!(active.frame, ReferenceFrame::LocalNed);
        let q_norm = (active.qw * active.qw
            + active.qx * active.qx
            + active.qy * active.qy
            + active.qz * active.qz)
            .sqrt();
        assert!((q_norm - 1.0).abs() < 1e-5);
        assert!((active.thrust - 0.5).abs() < 1e-5);

        let ControlIntent::AttitudeThrust(cutoff) =
            project_thrust_vector_ned([0.0, 0.0, 0.0], 19.6, 39.2, true)
        else {
            panic!("expected cutoff attitude thrust intent");
        };
        assert_eq!(cutoff.thrust, 0.0);

        let ControlIntent::AttitudeThrust(bank) = project_entry_bank_ned(0.6, FRAC_PI_2) else {
            panic!("expected entry bank attitude thrust intent");
        };
        assert_eq!(bank.frame, ReferenceFrame::LocalNed);
        assert_eq!(bank.thrust, 0.0);
        let bank_norm =
            (bank.qw * bank.qw + bank.qx * bank.qx + bank.qy * bank.qy + bank.qz * bank.qz).sqrt();
        assert!((bank_norm - 1.0).abs() < 1e-5);

        let limits = MissionLimits::default();
        let ControlIntent::Velocity(cart_ned) =
            project_cartesian_velocity_ned([1.5, 0.0, -0.4], 0.0, &limits, 1.0)
        else {
            panic!("expected velocity intent from cartesian NED");
        };
        assert_eq!(cart_ned.frame, ReferenceFrame::BodyFrd);
        assert!((cart_ned.vx - 1.5).abs() < 1e-5);
        assert!((cart_ned.vz + 0.4).abs() < 1e-5);

        let ControlIntent::Velocity(cart_body) =
            project_cartesian_velocity_body([0.8, -0.6, 0.2], 0.15, &limits)
        else {
            panic!("expected velocity intent from cartesian body");
        };
        assert_eq!(cart_body.frame, ReferenceFrame::BodyFrd);
        assert!((cart_body.vx - 0.8).abs() < 1e-5);
        assert!((cart_body.vy + 0.6).abs() < 1e-5);
        assert!((cart_body.vz - 0.2).abs() < 1e-5);
        assert!((cart_body.yaw_rate - 0.15).abs() < 1e-5);
    }
}
