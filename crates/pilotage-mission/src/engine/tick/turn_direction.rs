//! Mandatory turn direction enforcement on course, fix, and hold legs.

use core::f64::consts::{FRAC_PI_3, FRAC_PI_4, PI};

use navigate_contract::{GuidanceSetpoint, NavigationSolution, TurnDirection};
use navigate_route::{HoldEntry, HoldStage, HoldStatus, Leg};

use super::super::MissionEngine;

/// Angular lead (120°) commanded in the mandatory turn direction while the
/// desired course still lies on the opposite shorter side of the vehicle's
/// heading.
const FORCED_TURN_LEAD_RAD: f64 = 2.0 * FRAC_PI_3;

/// Antipodal deadband threshold (150°) above which the turn sense stays
/// locked to the mandatory direction rather than left to cross-track noise.
const ANTIPODAL_DEADBAND_RAD: f64 = 5.0 * PI / 6.0;

/// Small-angle tolerance (2°) inside which the vehicle is already aligned
/// with the desired course regardless of sign.
const ALIGNED_TOLERANCE_RAD: f64 = 2.0 * PI / 180.0;

/// Minimum groundspeed (m/s) at which solution ground course can substitute
/// for missing body yaw.
const MIN_COURSE_SPEED_MPS: f64 = 0.5;

/// Key identifying one leg or hold segment whose initial turn has been
/// satisfied.
pub(in crate::engine) type TurnStageKey = (usize, Option<HoldStage>);

/// Resolves the mandatory turn direction for the active `leg` and
/// `hold_status`.
pub(super) fn active_mandatory_turn(
    leg: &Leg<'_>,
    hold_status: Option<HoldStatus>,
) -> Option<TurnDirection> {
    let Some(hold) = leg.path.hold() else {
        return leg.to.turn_direction;
    };
    match hold_status.map(|s| s.stage).unwrap_or(HoldStage::ToFix) {
        HoldStage::ToFix => leg.to.turn_direction,
        HoldStage::EntryOutbound {
            entry: HoldEntry::Parallel,
        } => Some(opposite(hold.turn)),
        HoldStage::Inbound
            if hold_status.is_some_and(|s| {
                s.entry == Some(HoldEntry::Parallel) && s.circuits_completed == 0
            }) =>
        {
            Some(opposite(hold.turn))
        }
        HoldStage::EntryOutbound { .. } | HoldStage::Outbound | HoldStage::Inbound => {
            Some(hold.turn)
        }
    }
}

impl MissionEngine {
    /// Biases `setpoint` in `turn`'s direction when the vehicle's actual
    /// heading has not yet entered the forward capture sector for `key`.
    pub(super) fn enforce_mandatory_turn(
        &mut self,
        setpoint: GuidanceSetpoint,
        solution: &NavigationSolution,
        turn_spec: Option<(TurnStageKey, Option<TurnDirection>)>,
    ) -> GuidanceSetpoint {
        let Some((key, mandatory)) = turn_spec else {
            return setpoint;
        };
        let Some(dir) = mandatory else {
            self.turn_satisfied = Some(key);
            return setpoint;
        };
        if self.turn_satisfied == Some(key) {
            return setpoint;
        }
        let Some(heading_rad) = self.actual_heading_or_course(solution) else {
            return setpoint;
        };
        let Some(desired_rad) = setpoint_course(&setpoint) else {
            return setpoint;
        };
        let diff = wrap(desired_rad - heading_rad);
        let aligned = diff.abs() <= ALIGNED_TOLERANCE_RAD
            || (dir.sign() * diff >= 0.0 && diff.abs() <= FRAC_PI_4);
        if aligned {
            self.turn_satisfied = Some(key);
            return setpoint;
        }
        if dir.sign() * diff < 0.0 || diff.abs() > ANTIPODAL_DEADBAND_RAD {
            let forced_course = heading_rad + dir.sign() * FORCED_TURN_LEAD_RAD;
            return redirect_setpoint(setpoint, forced_course);
        }
        setpoint
    }

    fn actual_heading_or_course(&self, solution: &NavigationSolution) -> Option<f64> {
        if let Some(yaw) = self.last_yaw_rad {
            return Some(yaw);
        }
        let speed = solution
            .velocity
            .north_mps
            .hypot(solution.velocity.east_mps);
        (speed.is_finite() && speed >= MIN_COURSE_SPEED_MPS).then(|| {
            solution
                .velocity
                .east_mps
                .atan2(solution.velocity.north_mps)
        })
    }
}

const fn opposite(turn: TurnDirection) -> TurnDirection {
    match turn {
        TurnDirection::Left => TurnDirection::Right,
        TurnDirection::Right => TurnDirection::Left,
    }
}

fn setpoint_course(setpoint: &GuidanceSetpoint) -> Option<f64> {
    match setpoint {
        GuidanceSetpoint::Velocity { velocity } => {
            let speed = velocity.north_mps.hypot(velocity.east_mps);
            (speed.is_finite() && speed > 0.0).then(|| velocity.east_mps.atan2(velocity.north_mps))
        }
        GuidanceSetpoint::DeviationTracking { course_rad, .. } => Some(*course_rad),
        _ => None,
    }
}

fn redirect_setpoint(setpoint: GuidanceSetpoint, course_rad: f64) -> GuidanceSetpoint {
    match setpoint {
        GuidanceSetpoint::Velocity { mut velocity } => {
            let speed = velocity.north_mps.hypot(velocity.east_mps);
            if speed.is_finite() && speed > 0.0 {
                let (sin, cos) = course_rad.sin_cos();
                velocity.north_mps = speed * cos;
                velocity.east_mps = speed * sin;
            }
            GuidanceSetpoint::Velocity { velocity }
        }
        GuidanceSetpoint::DeviationTracking { vertical_m, .. } => {
            GuidanceSetpoint::DeviationTracking {
                lateral_m: 0.0,
                vertical_m,
                course_rad,
            }
        }
        other => other,
    }
}

fn wrap(angle: f64) -> f64 {
    angle.sin().atan2(angle.cos())
}
