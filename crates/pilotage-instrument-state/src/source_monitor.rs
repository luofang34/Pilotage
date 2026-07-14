//! End-to-end wiring of per-function source comparison (SRC-01) into display
//! resolution and the ALR-01 alert path.
//!
//! A display carries one [`SourceMonitors`] holding a comparator per display
//! function. [`resolve_with_sources`] runs them alongside the ordinary
//! resolve, folds the selected source and reversion state into [`PanelData`]
//! so a panel can annunciate which source it shows, and hands back the typed
//! miscompare transitions for the caller to forward into the alert manager.
//! The comparison logic and its guarantees live in
//! [`crate::source_compare`]; this module only connects it to the resolve and
//! alert seams.

use pilotage_alerts::{AlertEvent, MiscompareFault};
use pilotage_frames::Quat;

use crate::resolve::resolve_stateful;
use crate::source_compare::{
    AirframeSourcePolicy, AttitudeMeasure, Candidate, HeadingMeasure, ScalarMeasure,
    SourceAltitude, SourceComparator, SourceComparison,
};
use crate::{
    AircraftState, AirframeDisplayProfile, FreshnessPolicy, PanelData, UnusualAttitudeState,
};

mod sourced;
use sourced::sourced_function;
pub use sourced::{Sourced, SourcedFunction};

/// The per-function selected value, bound to its source, folded into
/// [`PanelData`]. Each field carries the selected source's own value, so a
/// panel cannot render one source's value under another's label.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SourceSelection {
    /// Attitude selection (the selected source's orientation).
    pub attitude: SourcedFunction<Quat>,
    /// Heading selection (radians from the selected source).
    pub heading: SourcedFunction<f32>,
    /// Altitude selection (meters from the selected source).
    pub altitude: SourcedFunction<f32>,
    /// Airspeed selection (the selected source's value).
    pub airspeed: SourcedFunction<f32>,
}

/// Candidate sources for one resolve step, one slice per display function.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceInputs<'a> {
    /// Attitude candidates.
    pub attitude: &'a [Candidate<AttitudeMeasure>],
    /// Heading candidates.
    pub heading: &'a [Candidate<HeadingMeasure>],
    /// Altitude candidates.
    pub altitude: &'a [Candidate<SourceAltitude>],
    /// Airspeed candidates.
    pub airspeed: &'a [Candidate<ScalarMeasure>],
}

/// The validated per-function selection policies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourcePolicies {
    /// Attitude policy.
    pub attitude: AirframeSourcePolicy,
    /// Heading policy.
    pub heading: AirframeSourcePolicy,
    /// Altitude policy.
    pub altitude: AirframeSourcePolicy,
    /// Airspeed policy.
    pub airspeed: AirframeSourcePolicy,
}

impl SourcePolicies {
    /// The simulator benchmark policies for every function. Benchmark data
    /// only; implies no aircraft approval.
    #[must_use]
    pub fn simulator() -> Self {
        Self {
            attitude: AirframeSourcePolicy::simulator(MiscompareFault::Attitude),
            heading: AirframeSourcePolicy::simulator(MiscompareFault::Heading),
            altitude: AirframeSourcePolicy::simulator(MiscompareFault::Altitude),
            airspeed: AirframeSourcePolicy::simulator(MiscompareFault::Airspeed),
        }
    }
}

/// The four per-function comparators a display carries across frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceMonitors {
    attitude: SourceComparator,
    heading: SourceComparator,
    altitude: SourceComparator,
    airspeed: SourceComparator,
}

impl Default for SourceMonitors {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceMonitors {
    /// Fresh comparators for every display function.
    #[must_use]
    pub fn new() -> Self {
        Self {
            attitude: SourceComparator::new(MiscompareFault::Attitude),
            heading: SourceComparator::new(MiscompareFault::Heading),
            altitude: SourceComparator::new(MiscompareFault::Altitude),
            airspeed: SourceComparator::new(MiscompareFault::Airspeed),
        }
    }

    /// Advances every function's comparator one step.
    pub fn step(
        &mut self,
        inputs: &SourceInputs,
        policies: &SourcePolicies,
        now_ms: u64,
    ) -> SourceMonitorReport {
        SourceMonitorReport {
            attitude: self
                .attitude
                .step(inputs.attitude, &policies.attitude, now_ms),
            heading: self.heading.step(inputs.heading, &policies.heading, now_ms),
            altitude: self
                .altitude
                .step(inputs.altitude, &policies.altitude, now_ms),
            airspeed: self
                .airspeed
                .step(inputs.airspeed, &policies.airspeed, now_ms),
        }
    }
}

/// The per-function comparison results for one monitored step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceMonitorReport {
    /// Attitude comparison.
    pub attitude: SourceComparison,
    /// Heading comparison.
    pub heading: SourceComparison,
    /// Altitude comparison.
    pub altitude: SourceComparison,
    /// Airspeed comparison.
    pub airspeed: SourceComparison,
}

impl SourceMonitorReport {
    /// The ALR-01 transitions to forward to the alert manager this step; a
    /// `None` entry carries no edge.
    #[must_use]
    pub fn transitions(&self) -> [Option<AlertEvent>; 4] {
        [
            self.attitude.transition,
            self.heading.transition,
            self.altitude.transition,
            self.airspeed.transition,
        ]
    }
}

/// One monitored resolve step's inputs: candidate sources, their policies,
/// and the caller's monotonic time.
#[derive(Debug, Clone, Copy)]
pub struct SourceStep<'a> {
    /// Candidate sources per function.
    pub inputs: SourceInputs<'a>,
    /// Per-function selection policies.
    pub policies: &'a SourcePolicies,
    /// Caller-supplied monotonic time in milliseconds.
    pub now_ms: u64,
}

/// Resolves display state and the source selection together: runs the
/// comparators over `sources`, resolves the base [`PanelData`], and folds the
/// selected source and reversion state into it. The returned
/// [`SourceMonitorReport`] carries the ALR-01 transitions to forward to the
/// alert manager.
pub fn resolve_with_sources(
    state: &AircraftState,
    policy: &FreshnessPolicy,
    profile: &AirframeDisplayProfile,
    unusual: &mut UnusualAttitudeState,
    monitors: &mut SourceMonitors,
    sources: &SourceStep,
) -> (PanelData, SourceMonitorReport) {
    let report = monitors.step(&sources.inputs, sources.policies, sources.now_ms);
    let mut panel = resolve_stateful(state, policy, profile, unusual);
    panel.sources = SourceSelection {
        attitude: sourced_function(sources.inputs.attitude, &report.attitude),
        heading: sourced_function(sources.inputs.heading, &report.heading),
        altitude: sourced_function(sources.inputs.altitude, &report.altitude),
        airspeed: sourced_function(sources.inputs.airspeed, &report.airspeed),
    };
    (panel, report)
}

#[cfg(test)]
mod tests;
