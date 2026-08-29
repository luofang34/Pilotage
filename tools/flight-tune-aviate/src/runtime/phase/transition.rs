//! Reaching and holding the declared test start state.
//!
//! Every start state is relative to the first frame after the simulator
//! reset. This module latches that frame once per run and resolves the
//! declared target against it, so two runs of one mission measure the same
//! geometry whatever absolute position the simulator chose.

use flight_tune::{ScenarioFrame, StartHeading, StartState};

use crate::runtime::AviateRuntimeError;
use crate::runtime::math::{distance_m, heading_error_rad, magnitude, yaw_rad};
use crate::runtime::timing::{FrameStamp, PhaseClock};

/// How close the vehicle has to be to count as at the start state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StartStateTolerance {
    /// The largest permitted position error in meters.
    pub position_m: f64,
    /// The largest permitted heading error in radians.
    pub heading_rad: f64,
    /// The largest permitted speed while settled, in meters per second.
    pub speed_mps: f64,
    /// The continuous trial time the vehicle has to hold the state.
    pub dwell_ns: u64,
}

impl StartStateTolerance {
    /// Rejects a tolerance that no vehicle can satisfy or that accepts any state.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a bound is not finite or positive.
    pub fn validate(&self) -> Result<(), AviateRuntimeError> {
        for (field, value) in [
            ("start position tolerance", self.position_m),
            ("start heading tolerance", self.heading_rad),
            ("start speed tolerance", self.speed_mps),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(AviateRuntimeError::InvalidValue { field });
            }
        }
        if self.dwell_ns == 0 {
            return Err(AviateRuntimeError::InvalidValue {
                field: "start dwell",
            });
        }
        Ok(())
    }
}

/// The reset-relative origin that every start state measures from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StartOrigin {
    position_ned_m: Option<[f64; 3]>,
    heading_rad: Option<f64>,
}

impl StartOrigin {
    /// Creates one origin that no frame has latched.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            position_ned_m: None,
            heading_rad: None,
        }
    }

    /// Latches the reset-relative origin from the first frame of a run.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the frame attitude is invalid.
    pub fn latch(&mut self, frame: &ScenarioFrame) -> Result<(), AviateRuntimeError> {
        if self.position_ned_m.is_none() {
            self.position_ned_m = Some(frame.truth.position_ned_m);
            self.heading_rad = Some(yaw_rad(frame.truth.attitude_wxyz)?);
        }
        Ok(())
    }

    /// Clears the latched origin so the next run latches its own.
    pub const fn clear(&mut self) {
        self.position_ned_m = None;
        self.heading_rad = None;
    }

    /// The absolute position that one declared start state names.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when no frame has latched the origin.
    pub fn target_position_ned_m(
        &self,
        target: &StartState,
    ) -> Result<[f64; 3], AviateRuntimeError> {
        let origin = self
            .position_ned_m
            .ok_or(AviateRuntimeError::NoStartOrigin)?;
        let mut absolute = [0.0; 3];
        for index in 0..3 {
            absolute[index] = origin[index] + target.relative_position_ned_m[index];
        }
        Ok(absolute)
    }

    /// The absolute heading that one declared start state names.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when no frame has latched the origin.
    pub fn target_heading_rad(&self, target: &StartState) -> Result<f64, AviateRuntimeError> {
        let origin = self.heading_rad.ok_or(AviateRuntimeError::NoStartOrigin)?;
        Ok(match target.heading {
            StartHeading::True { radians } => radians,
            StartHeading::ResetOffset { radians } => origin + radians,
        })
    }
}

/// How far one frame is from the declared start state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StartStateError {
    /// The position error in meters.
    pub position_m: f64,
    /// The signed shortest heading error in radians.
    pub heading_rad: f64,
    /// The speed in meters per second.
    pub speed_mps: f64,
}

impl StartStateError {
    /// Whether this error is inside one declared tolerance.
    #[must_use]
    pub fn is_within(&self, tolerance: &StartStateTolerance) -> bool {
        self.position_m <= tolerance.position_m
            && self.heading_rad.abs() <= tolerance.heading_rad
            && self.speed_mps <= tolerance.speed_mps
    }
}

/// The distance from one frame to a declared start state.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when the origin is not latched or a frame
/// value is not finite.
pub fn start_state_error(
    origin: &StartOrigin,
    frame: &ScenarioFrame,
    target: &StartState,
) -> Result<StartStateError, AviateRuntimeError> {
    Ok(StartStateError {
        position_m: distance_m(
            frame.truth.position_ned_m,
            origin.target_position_ned_m(target)?,
        )?,
        heading_rad: heading_error_rad(
            yaw_rad(frame.truth.attitude_wxyz)?,
            origin.target_heading_rad(target)?,
        )?,
        speed_mps: magnitude(frame.truth.velocity_ned_mps)?,
    })
}

/// A dwell window that one continuous excursion resets.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SettleWindow {
    held: PhaseClock,
}

impl SettleWindow {
    /// Creates one window that no frame has opened.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            held: PhaseClock::new(),
        }
    }

    /// Clears the window so the next accepted frame opens a new one.
    pub const fn reset(&mut self) {
        self.held.leave();
    }

    /// Advances the window and reports whether the dwell is complete.
    ///
    /// A frame outside tolerance clears the window, so the dwell measures
    /// one continuous hold and never the sum of several short ones.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the frame time steps backward.
    pub fn advance(
        &mut self,
        stamp: FrameStamp,
        within: bool,
        dwell_ns: u64,
    ) -> Result<bool, AviateRuntimeError> {
        if !within {
            self.held.leave();
            return Ok(false);
        }
        self.held.enter(stamp);
        Ok(self.held.elapsed_ns(stamp)? >= dwell_ns)
    }
}
