//! The targets an operator selects: a true heading, an altitude, and an
//! indicated airspeed. Each one replaces what the plan gives for the same
//! quantity. A target that is not selected stays with the plan.
//!
//! Only a vehicle that cannot stop takes selected targets: the heading
//! and altitude demand is the one intent that carries them. The targets
//! apply while the vehicle is in flight under the mission: in its climb,
//! on its route, and after its plan. They do not apply before the arm or
//! while a human holds the motion scope.
//!
//! The selection is plain data. The engine reads it on each tick, so a
//! change applies from the next tick.

use core::f64::consts::TAU;

use navigate_contract::{AltitudeConstraint, MonotonicNanos, NavigationSolution, Waypoint};
use pilotage_protocol::ControlIntent;

use super::{MissionEngine, MissionEvent};

#[cfg(test)]
mod tests;

/// The targets an operator selects in place of the plan's.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SelectedTargets {
    /// The true heading to fly in place of the plan's lateral path,
    /// radians clockwise from true north.
    pub heading_true_rad: Option<f64>,
    /// The altitude above the ellipsoid to capture and hold in place of
    /// the plan's, metres.
    pub altitude_m: Option<f64>,
    /// The indicated airspeed to fly in place of the configured cruise
    /// speed, m/s.
    pub airspeed_mps: Option<f64>,
}

/// Why the engine refused a selection.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum SelectError {
    /// A selected target is not a finite number.
    #[error("the selected {target} is not a finite number: {value}")]
    NotFinite {
        /// The name of the target.
        target: &'static str,
        /// The value given.
        value: f64,
    },
    /// The selected airspeed is zero or less.
    #[error("the selected airspeed is not above zero: {value} m/s")]
    AirspeedNotPositive {
        /// The value given, m/s.
        value: f64,
    },
    /// The vehicle can stop. Its intents do not carry selected targets.
    #[error("a vehicle that can stop does not fly selected targets")]
    VehicleCanStop,
}

impl SelectedTargets {
    /// Whether no target is selected: the plan is flown.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.heading_true_rad.is_none() && self.altitude_m.is_none() && self.airspeed_mps.is_none()
    }

    /// This selection as the engine keeps it, for a vehicle that can stop
    /// (`can_stop`) or cannot: the heading is in `[0, 2π)`.
    ///
    /// # Errors
    ///
    /// [`SelectError::NotFinite`] for a target that is not a finite
    /// number, [`SelectError::AirspeedNotPositive`] for an airspeed of
    /// zero or less, and [`SelectError::VehicleCanStop`] for a selection
    /// with a target on a vehicle that can stop. A selection with no
    /// target is correct for each vehicle.
    pub fn checked(self, can_stop: bool) -> Result<Self, SelectError> {
        for (target, value) in [
            ("heading", self.heading_true_rad),
            ("altitude", self.altitude_m),
            ("airspeed", self.airspeed_mps),
        ] {
            if let Some(value) = value
                && !value.is_finite()
            {
                return Err(SelectError::NotFinite { target, value });
            }
        }
        if let Some(value) = self.airspeed_mps
            && value <= 0.0
        {
            return Err(SelectError::AirspeedNotPositive { value });
        }
        if can_stop && !self.is_empty() {
            return Err(SelectError::VehicleCanStop);
        }
        Ok(Self {
            heading_true_rad: self.heading_true_rad.map(heading_in_turn),
            ..self
        })
    }
}

/// `heading` in `[0, 2π)`. The remainder of a small negative angle rounds
/// to a full turn, which is north.
fn heading_in_turn(heading: f64) -> f64 {
    let wrapped = heading.rem_euclid(TAU);
    if wrapped < TAU { wrapped } else { 0.0 }
}

impl MissionEngine {
    /// Replaces the selection with `targets`. A selection with no target
    /// flies the plan. The change applies from the next tick, which
    /// reports it with [`MissionEvent::TargetsSelected`].
    ///
    /// # Errors
    ///
    /// The refusals of [`SelectedTargets::checked`]. A refused selection
    /// leaves the current one in effect.
    pub fn select(&mut self, targets: SelectedTargets) -> Result<(), SelectError> {
        let targets = targets.checked(self.config.can_stop())?;
        let current = self.modes.selected;
        if targets == current {
            return Ok(());
        }
        self.modes.selected = targets;
        if targets.airspeed_mps != current.airspeed_mps {
            // Guidance commands its own cruise speed and turns toward a
            // track by its gain for that speed.
            let cruise_mps = targets.airspeed_mps.unwrap_or(self.config.cruise_mps);
            super::shape_cruise(&mut self.guidance, &self.config, cruise_mps);
        }
        self.hold_events
            .push(MissionEvent::TargetsSelected { targets });
        Ok(())
    }

    /// The selection in effect. No target is selected until
    /// [`Self::select`] sets one.
    #[must_use]
    pub const fn selected(&self) -> SelectedTargets {
        self.modes.selected
    }

    /// The selection that applies now: the selection while the vehicle is
    /// in flight under the mission, and no target before that or while a
    /// human holds the motion scope.
    pub(super) fn flown_selection(&self) -> SelectedTargets {
        let lifecycle = self.lifecycle();
        if lifecycle.in_flight() && !lifecycle.suspended {
            self.modes.selected
        } else {
            SelectedTargets::default()
        }
    }

    /// Whether a selected heading replaces the plan's lateral path. The
    /// plan does not sequence while it does: the vehicle does not fly the
    /// plan's legs, so a leg it flies past is not a leg it flew.
    pub(super) const fn heading_selected(&self) -> bool {
        self.modes.selected.heading_true_rad.is_some()
    }

    /// `to` at the selected altitude, or `to` unchanged with no altitude
    /// selected. Guidance takes its vertical target from the altitude
    /// constraint, so the constraint changes with the position.
    pub(super) fn at_selected_altitude(&self, to: &Waypoint) -> Waypoint {
        let mut to = *to;
        if let Some(altitude_m) = self.flown_selection().altitude_m {
            to.position.altitude_m = altitude_m;
            to.altitude = Some(AltitudeConstraint::At(altitude_m));
        }
        to
    }

    /// The intent of the selected heading: that heading at the vertical
    /// target, or at the present altitude with no target. `None` with no
    /// heading selected or no altitude to hold.
    pub(super) fn selected_heading_intent(
        &mut self,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        let heading = self.modes.selected.heading_true_rad?;
        let altitude_m = self
            .modes
            .vertical
            .target_m
            .or_else(|| solution.map(|solution| solution.position.altitude_m))?;
        // The pattern after the plan is entered again from where the
        // selected heading leaves the vehicle, not continued from where
        // the selection took it.
        self.holding_pattern = None;
        Some(self.steer_selected(solution, heading, altitude_m, now, events))
    }
}
