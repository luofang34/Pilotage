//! The flight verifier.
//!
//! Its inputs are the scenario's expectation, simulator truth, and the
//! flight controller's armed report. It never sees the classifier's
//! answer or the sequencer's phase. If the classifier misreads a message,
//! the vehicle flies to the wrong place and this module fails the run.
//! With no truth on the wire the run is inconclusive, never a pass.

use serde::Serialize;

use crate::scenario::{Arrival, Scenario, Waypoint};
use crate::vehicle_state::TruthState;

/// Truth height that proves the vehicle left the ground, in metres.
const AIRBORNE_M: f64 = 1.0;
/// Truth height at or below which the vehicle is on the ground.
const ON_GROUND_M: f64 = 0.5;
/// Truth height tolerance for a hold at cruise height, in metres.
const HOLD_HEIGHT_TOLERANCE_M: f64 = 1.5;
/// Time the end condition must stay true, in seconds.
const PROOF_S: f64 = 3.0;
/// Movement that breaks an at-rest proof, in metres.
const REST_TOLERANCE_M: f64 = 0.3;

/// The result class of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub(crate) enum Verdict {
    /// Truth shows the expected end state, held for the proof time.
    Pass,
    /// Truth shows a violation, or the deadline passed.
    Fail,
    /// No simulator truth arrived, so nothing can be shown.
    Inconclusive,
}

/// The verifier's findings, written to the run record.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Report {
    /// The result class.
    pub verdict: Verdict,
    /// Why, in words.
    pub reasons: Vec<String>,
    /// Seconds from the first message to the verdict.
    pub elapsed_s: f64,
    /// Truth samples seen.
    pub truth_samples: u64,
    /// Greatest truth height, when the vehicle reports height.
    pub max_height_m: Option<f64>,
    /// Closest truth approach to the expected final waypoint.
    pub closest_to_final_m: Option<f64>,
}

/// Judges one flight against one scenario.
#[derive(Debug)]
pub(crate) struct Verifier {
    final_name: String,
    final_waypoint: Waypoint,
    final_on_arrival: Arrival,
    forbidden: Vec<(String, Waypoint)>,
    approaches: Vec<(String, Waypoint, f64)>,
    next_approach: usize,
    radius_m: f64,
    cruise_height_m: f64,
    timeout_s: f64,
    truth_samples: u64,
    max_height_m: Option<f64>,
    closest_m: Option<f64>,
    violations: Vec<String>,
    proof_anchor: Option<(f64, TruthState)>,
}

impl Verifier {
    /// A verifier for `scenario`, or `None` when the scenario names a
    /// waypoint it does not define. `Scenario::load` rejects those.
    pub(crate) fn new(scenario: &Scenario) -> Option<Self> {
        let expect = &scenario.expect;
        let forbidden = expect
            .must_not_reach
            .iter()
            .map(|name| Some((name.clone(), scenario.position(name)?)))
            .collect::<Option<Vec<_>>>()?;
        let approaches = expect
            .must_approach
            .iter()
            .map(|approach| {
                let waypoint = scenario.position(&approach.target)?;
                Some((approach.target.clone(), waypoint, approach.within_m))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            approaches,
            next_approach: 0,
            final_name: expect.final_target.clone(),
            final_waypoint: scenario.position(&expect.final_target)?,
            final_on_arrival: expect.final_on_arrival,
            forbidden,
            radius_m: scenario.arrival_radius_m,
            cruise_height_m: scenario.cruise_height_m,
            timeout_s: expect.timeout_s,
            truth_samples: 0,
            max_height_m: None,
            closest_m: None,
            violations: Vec::new(),
            proof_anchor: None,
        })
    }

    /// Takes one truth sample and the flight controller's armed report.
    /// Returns the report as soon as the run is decided.
    pub(crate) fn observe(
        &mut self,
        truth: &TruthState,
        armed: Option<bool>,
        elapsed_s: f64,
    ) -> Option<Report> {
        self.truth_samples = self.truth_samples.wrapping_add(1);
        if let Some(height) = truth.height_m {
            self.max_height_m = Some(self.max_height_m.map_or(height, |max| max.max(height)));
        }
        let range = range_m(truth, self.final_waypoint);
        self.closest_m = Some(self.closest_m.map_or(range, |closest| closest.min(range)));
        for (name, waypoint) in &self.forbidden {
            if range_m(truth, *waypoint) <= self.radius_m {
                self.violations
                    .push(format!("truth entered the arrival radius of {name}"));
                return Some(self.report(Verdict::Fail, elapsed_s));
            }
        }
        if let Some((_, waypoint, within_m)) = self.approaches.get(self.next_approach)
            && range_m(truth, *waypoint) <= *within_m
        {
            self.next_approach = self.next_approach.wrapping_add(1);
        }
        if !self.end_state_holds(truth, armed, range) {
            self.proof_anchor = None;
            return None;
        }
        let (since, anchor) = *self.proof_anchor.get_or_insert((elapsed_s, *truth));
        if self.final_on_arrival == Arrival::Land
            && (truth.north_m - anchor.north_m).hypot(truth.east_m - anchor.east_m)
                > REST_TOLERANCE_M
        {
            // A landed vehicle is at rest. One that still moves is not.
            self.proof_anchor = Some((elapsed_s, *truth));
            return None;
        }
        (elapsed_s - since >= PROOF_S).then(|| self.report(Verdict::Pass, elapsed_s))
    }

    /// The report at the deadline, or `None` before it.
    pub(crate) fn on_clock(&mut self, elapsed_s: f64) -> Option<Report> {
        if elapsed_s < self.timeout_s {
            return None;
        }
        if self.truth_samples == 0 {
            self.violations
                .push("no simulator truth arrived before the deadline".to_owned());
            return Some(self.report(Verdict::Inconclusive, elapsed_s));
        }
        if let Some((name, _, within_m)) = self.approaches.get(self.next_approach) {
            self.violations
                .push(format!("truth never came within {within_m} m of {name}"));
        }
        self.violations.push(format!(
            "the end state at {} was not shown in {} s",
            self.final_name, self.timeout_s
        ));
        Some(self.report(Verdict::Fail, elapsed_s))
    }

    fn end_state_holds(&self, truth: &TruthState, armed: Option<bool>, range_m: f64) -> bool {
        if range_m > self.radius_m || self.next_approach < self.approaches.len() {
            return false;
        }
        // A vehicle that reports height must have flown. A vehicle that
        // never left the ground at HOME has not completed a flight there.
        let flew = self
            .max_height_m
            .is_none_or(|max_height| max_height >= AIRBORNE_M);
        match self.final_on_arrival {
            Arrival::Land => {
                let down = truth.height_m.is_none_or(|height| height <= ON_GROUND_M);
                flew && down && armed != Some(true)
            }
            Arrival::Hold => {
                let level = truth.height_m.is_none_or(|height| {
                    (height - self.cruise_height_m).abs() <= HOLD_HEIGHT_TOLERANCE_M
                });
                flew && level
            }
        }
    }

    fn report(&self, verdict: Verdict, elapsed_s: f64) -> Report {
        let mut reasons = self.violations.clone();
        if verdict == Verdict::Pass {
            reasons.push(format!(
                "truth held the {:?} end state at {} for {PROOF_S} s",
                self.final_on_arrival, self.final_name
            ));
        }
        Report {
            verdict,
            reasons,
            elapsed_s,
            truth_samples: self.truth_samples,
            max_height_m: self.max_height_m,
            closest_to_final_m: self.closest_m,
        }
    }
}

fn range_m(truth: &TruthState, waypoint: Waypoint) -> f64 {
    (waypoint.north_m - truth.north_m).hypot(waypoint.east_m - truth.east_m)
}

#[cfg(test)]
mod tests;
