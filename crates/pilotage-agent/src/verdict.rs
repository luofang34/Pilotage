//! The flight verifier.
//!
//! Its inputs are the scenario's expectation, truth, and the flight
//! controller's armed report. It never sees a model reply or the executor
//! phase. If a model misreads a message, the vehicle does the wrong thing and
//! this module fails the run. With no truth the run is inconclusive, never a
//! pass.

use serde::Serialize;

use crate::scenario::{Checkpoint, EndState, Fix, Scenario};
use crate::state::TruthState;

/// Truth height that proves the vehicle left the ground, in metres.
const AIRBORNE_M: f64 = 1.0;
/// Truth height at or below which the vehicle is on the ground.
const ON_GROUND_M: f64 = 0.5;
/// Truth height tolerance for a hold at the cruise height, in metres.
const HOLD_HEIGHT_TOLERANCE_M: f64 = 1.5;
/// Time that the end state must stay true, in seconds.
const PROOF_S: f64 = 3.0;
/// Movement that breaks an at-rest proof, in metres.
const REST_TOLERANCE_M: f64 = 0.3;

/// The result class of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Verdict {
    /// Truth shows the expected result.
    Pass,
    /// Truth shows a violation, or the deadline passed.
    Fail,
    /// No truth arrived, so nothing can be shown.
    Inconclusive,
}

/// The verifier's findings, written to the run record.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
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
    /// Checkpoints that truth showed, of the number expected.
    pub checkpoints_met: usize,
}

/// Judges one flight against one scenario.
#[derive(Debug)]
pub struct Verifier {
    checkpoints: Vec<Checkpoint>,
    next_checkpoint: usize,
    held_since_s: Option<f64>,
    end: EndState,
    end_fix: Option<Fix>,
    forbidden: Vec<(String, Fix)>,
    fixes: Vec<(String, Fix)>,
    radius_m: f64,
    cruise_height_m: f64,
    timeout_s: f64,
    truth_samples: u64,
    max_height_m: Option<f64>,
    violations: Vec<String>,
    proof_anchor: Option<(f64, TruthState)>,
}

impl Verifier {
    /// A verifier for a scenario that `Scenario::parse` accepted. It returns
    /// `None` when the scenario names a fix that it does not define.
    #[must_use]
    pub fn new(scenario: &Scenario) -> Option<Self> {
        let expect = &scenario.expect;
        let named = |name: &String| Some((name.clone(), scenario.position(name)?));
        let end_fix = match &expect.end {
            EndState::LandedAt { fix } | EndState::HoldingAt { fix } => {
                Some(scenario.position(fix)?)
            }
            EndState::CheckpointsOnly => None,
        };
        let mut fixes = Vec::new();
        for checkpoint in &expect.checkpoints {
            if let Checkpoint::Approach { fix, .. } = checkpoint {
                fixes.push(named(fix)?);
            }
        }
        Some(Self {
            checkpoints: expect.checkpoints.clone(),
            next_checkpoint: 0,
            held_since_s: None,
            end: expect.end.clone(),
            end_fix,
            forbidden: expect
                .must_not_reach
                .iter()
                .map(named)
                .collect::<Option<Vec<_>>>()?,
            fixes,
            radius_m: scenario.arrival_radius_m,
            cruise_height_m: scenario.cruise_height_m,
            timeout_s: expect.timeout_s,
            truth_samples: 0,
            max_height_m: None,
            violations: Vec::new(),
            proof_anchor: None,
        })
    }

    /// Takes one truth sample and the flight controller's armed report.
    /// Returns the report as soon as the run is decided.
    pub fn observe(
        &mut self,
        truth: &TruthState,
        armed: Option<bool>,
        elapsed_s: f64,
    ) -> Option<Report> {
        self.truth_samples = self.truth_samples.wrapping_add(1);
        if let Some(height) = truth.height_m {
            self.max_height_m = Some(self.max_height_m.map_or(height, |max| max.max(height)));
        }
        for (name, fix) in &self.forbidden {
            if range_m(truth, *fix) <= self.radius_m {
                self.violations
                    .push(format!("truth entered the arrival radius of {name}"));
                return Some(self.report(Verdict::Fail, elapsed_s));
            }
        }
        self.advance_checkpoints(truth, elapsed_s);
        if self.next_checkpoint < self.checkpoints.len() || !self.flew() {
            self.proof_anchor = None;
            return None;
        }
        if self.end == EndState::CheckpointsOnly {
            return Some(self.report(Verdict::Pass, elapsed_s));
        }
        if !self.end_state_holds(truth, armed) {
            self.proof_anchor = None;
            return None;
        }
        let (since, anchor) = *self.proof_anchor.get_or_insert((elapsed_s, *truth));
        let moved = (truth.north_m - anchor.north_m).hypot(truth.east_m - anchor.east_m);
        if matches!(self.end, EndState::LandedAt { .. }) && moved > REST_TOLERANCE_M {
            // A landed vehicle is at rest. One that still moves is not.
            self.proof_anchor = Some((elapsed_s, *truth));
            return None;
        }
        (elapsed_s - since >= PROOF_S).then(|| self.report(Verdict::Pass, elapsed_s))
    }

    /// The report at the deadline, or `None` before it.
    pub fn on_clock(&mut self, elapsed_s: f64) -> Option<Report> {
        if elapsed_s < self.timeout_s {
            return None;
        }
        if self.truth_samples == 0 {
            self.violations
                .push("no truth arrived before the deadline".to_owned());
            return Some(self.report(Verdict::Inconclusive, elapsed_s));
        }
        if let Some(checkpoint) = self.checkpoints.get(self.next_checkpoint) {
            self.violations
                .push(format!("truth never showed the checkpoint {checkpoint:?}"));
        }
        self.violations
            .push(format!("the result was not shown in {} s", self.timeout_s));
        Some(self.report(Verdict::Fail, elapsed_s))
    }

    /// A vehicle that reports height must have flown. A vehicle that never
    /// left the ground at `HOME` has not completed a flight there.
    fn flew(&self) -> bool {
        self.max_height_m
            .is_none_or(|max_height| max_height >= AIRBORNE_M)
    }

    fn advance_checkpoints(&mut self, truth: &TruthState, elapsed_s: f64) {
        let Some(checkpoint) = self.checkpoints.get(self.next_checkpoint) else {
            return;
        };
        let (holds, for_s) = match checkpoint {
            Checkpoint::Approach { fix, within_m } => {
                let near = self
                    .fixes
                    .iter()
                    .find(|(name, _)| name == fix)
                    .is_some_and(|(_, at)| range_m(truth, *at) <= *within_m);
                (near, 0.0)
            }
            Checkpoint::HeadingHeld {
                degrees,
                tolerance_deg,
                for_s,
            } => {
                let held = truth.yaw_rad.is_some_and(|yaw| {
                    let error = (yaw.to_degrees() - degrees + 180.0).rem_euclid(360.0) - 180.0;
                    error.abs() <= *tolerance_deg
                });
                (held, *for_s)
            }
            Checkpoint::HeightHeld {
                height_m,
                tolerance_m,
                for_s,
            } => {
                let held = truth
                    .height_m
                    .is_some_and(|height| (height - height_m).abs() <= *tolerance_m);
                (held, *for_s)
            }
        };
        if !holds {
            self.held_since_s = None;
            return;
        }
        let since = *self.held_since_s.get_or_insert(elapsed_s);
        if elapsed_s - since >= for_s {
            self.next_checkpoint = self.next_checkpoint.wrapping_add(1);
            self.held_since_s = None;
        }
    }

    fn end_state_holds(&self, truth: &TruthState, armed: Option<bool>) -> bool {
        let Some(fix) = self.end_fix else {
            return true;
        };
        if range_m(truth, fix) > self.radius_m {
            return false;
        }
        match self.end {
            EndState::LandedAt { .. } => {
                let down = truth.height_m.is_none_or(|height| height <= ON_GROUND_M);
                down && armed != Some(true)
            }
            EndState::HoldingAt { .. } => truth.height_m.is_none_or(|height| {
                (height - self.cruise_height_m).abs() <= HOLD_HEIGHT_TOLERANCE_M
            }),
            EndState::CheckpointsOnly => true,
        }
    }

    fn report(&self, verdict: Verdict, elapsed_s: f64) -> Report {
        let mut reasons = self.violations.clone();
        if verdict == Verdict::Pass {
            reasons.push(format!(
                "truth showed {} checkpoints and the end state {:?}",
                self.checkpoints.len(),
                self.end
            ));
        }
        Report {
            verdict,
            reasons,
            elapsed_s,
            truth_samples: self.truth_samples,
            max_height_m: self.max_height_m,
            checkpoints_met: self.next_checkpoint,
        }
    }
}

fn range_m(truth: &TruthState, fix: Fix) -> f64 {
    (fix.north_m - truth.north_m).hypot(fix.east_m - truth.east_m)
}

#[cfg(test)]
mod tests;
