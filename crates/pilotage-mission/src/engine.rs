//! The sans-IO mission engine: snapshot + route in, typed intents and
//! actions out. All time is caller-supplied; nothing here reads a clock
//! or performs I/O.

mod nav_guidance;
mod output;
mod tick;

use core::mem::Discriminant;

use aerocontext_core::NavDataSnapshot;
use aerocontext_planning::route::expand_str;
use navigate_contract::{
    AltitudeConstraint, FlightPlan, GeodeticPosition, MonotonicNanos, NavigationSolution,
    ObservationStamp, PlanRole, SensorClass, SourceComposition, SourceEpoch, SourceId,
    SymmetricCov3, Waypoint, WrappingSequence,
};
use navigate_fpl::{ExecutionConfig, PlanExecution};
use navigate_fusion::{FusionConfig, NavigationFilter, Observation, ObservationValue};
use navigate_geodesy::{LocalTangentPlane, NedOffset};
use navigate_guidance::{GuidanceRefusal, VelocityGuidanceConfig};

pub use nav_guidance::{NavGuidance, NavQuality};
pub use output::{MissionAction, MissionCounters, MissionEvent, MissionOutput, MissionState};

use crate::config::MissionConfig;
use crate::error::MissionBuildError;
use crate::ownship::{OwnshipSample, TruthRole};
use crate::provenance::{MissionPlanRecord, SnapshotProvenance};

/// The single synthesized-GNSS source identity the engine stamps its
/// observations with.
const OWNSHIP_SOURCE: SourceId = SourceId::new(1);

/// A deterministic mission executor over the Navigate stack.
///
/// The host owns the boundary: it converts telemetry into
/// [`OwnshipSample`]s, frames the emitted intents/actions onto the
/// session wire, and supplies every `now` on the configured clock
/// domain. The engine owns the judgment: admission, sequencing,
/// guidance, and the phase machine.
#[derive(Debug)]
pub struct MissionEngine {
    config: MissionConfig,
    plane: LocalTangentPlane,
    filter: NavigationFilter,
    execution: PlanExecution,
    guidance: VelocityGuidanceConfig,
    state: MissionState,
    counters: MissionCounters,
    /// The solution the last tick published, backing the display-facing
    /// guidance view. A tick whose filter published nothing clears it, so
    /// [`MissionEngine::nav_guidance`] cannot serve stale geometry.
    last_solution: Option<NavigationSolution>,
    pending_events: Vec<MissionEvent>,
    /// The latest known heading. `None` until a sample carries an
    /// attitude group: intents need the NED→body rotation, and guessing
    /// zero would silently rotate every command to due north.
    last_yaw_rad: Option<f64>,
    next_action_id: u64,
    outstanding_arm: Option<u64>,
    arm_needs_send: bool,
    last_refusal: Option<Discriminant<GuidanceRefusal>>,
}

impl MissionEngine {
    /// Expands the route against the snapshot, builds the mission plan,
    /// and returns the engine with its pack-for-flight record.
    ///
    /// Waypoint positions convert from the snapshot's degrees to
    /// radians exactly once, here (ADR-0030). Every waypoint carries an
    /// `At` constraint of anchor altitude plus the cruise height.
    ///
    /// # Errors
    ///
    /// Any [`MissionBuildError`]: expansion failure (with cycle
    /// context), an empty expansion, an implausible anchor, or a plan
    /// that fails structural validation.
    pub fn new(
        snapshot: &NavDataSnapshot,
        provenance: SnapshotProvenance,
        config: MissionConfig,
    ) -> Result<(Self, MissionPlanRecord), MissionBuildError> {
        let expanded = expand_str(&config.route, snapshot).map_err(|source| {
            MissionBuildError::RouteExpansion {
                route: config.route.clone(),
                cycle: snapshot.cycle.effective_on,
                source,
            }
        })?;
        if expanded.points.is_empty() {
            return Err(MissionBuildError::EmptyRoute {
                route: config.route.clone(),
            });
        }
        let waypoints = build_waypoints(&expanded.points, &config);
        let expanded_idents: Vec<String> = waypoints.iter().map(|wp| wp.ident.clone()).collect();
        let record = MissionPlanRecord {
            provenance,
            route_input: config.route.clone(),
            waypoint_count: waypoints.len(),
            expanded_idents,
        };
        let plan = FlightPlan::new(
            format!("mission:{}", config.route),
            PlanRole::Mission,
            waypoints,
        );
        let execution = PlanExecution::new(plan, ExecutionConfig::default())?;
        let plane = LocalTangentPlane::new(config.anchor)?;
        let filter = NavigationFilter::new(FusionConfig::default(), config.clock);
        let guidance = guidance_config(&config);
        let engine = Self {
            config,
            plane,
            filter,
            execution,
            guidance,
            state: MissionState::AwaitSolution,
            counters: MissionCounters::default(),
            last_solution: None,
            pending_events: Vec::new(),
            last_yaw_rad: None,
            next_action_id: 0,
            outstanding_arm: None,
            arm_needs_send: false,
            last_refusal: None,
        };
        Ok((engine, record))
    }

    /// Offers one ownship sample. Only [`TruthRole::SimulationTruth`]
    /// samples become observations — see [`OwnshipSample`] for why any
    /// other role is a counted refusal, never an aid. `now` is the
    /// admission reference on the engine's clock domain.
    pub fn on_ownship(&mut self, sample: &OwnshipSample, now: MonotonicNanos) {
        if sample.role != TruthRole::SimulationTruth {
            self.counters.rejected_role = self.counters.rejected_role.wrapping_add(1);
            return;
        }
        if let Some(yaw) = sample.yaw_rad {
            self.last_yaw_rad = Some(yaw);
        }
        let position =
            self.plane
                .from_ned(&NedOffset::new(sample.ned[0], sample.ned[1], sample.ned[2]));
        let variance = self.config.gnss_sigma_m * self.config.gnss_sigma_m;
        let stamp = ObservationStamp::new(
            OWNSHIP_SOURCE,
            // Epoch stays 0 until a reset/relaunch story exists; the
            // filter treats a new epoch as a new source incarnation.
            SourceEpoch::new(0),
            WrappingSequence::new(sample.sequence),
            sample.acquired_at,
            self.config.clock,
        );
        let observation = Observation::new(
            stamp,
            ObservationValue::PositionFix {
                position,
                covariance: SymmetricCov3::from_diagonal(variance, variance, variance),
            },
            SourceComposition::of(SensorClass::Gnss),
        );
        if !self.filter.ingest(&observation, now).is_accepted() {
            self.counters.fusion_rejected = self.counters.fusion_rejected.wrapping_add(1);
        }
    }

    /// Reports the correlated result of a previously emitted action.
    /// Acceptance advances the phase machine; rejection schedules a
    /// re-send with a fresh id on the next tick. Results for unknown
    /// ids, or outside the arming phase, are inert.
    pub fn on_action_result(&mut self, action_id: u64, accepted: bool) {
        if self.state != MissionState::Arming || self.outstanding_arm != Some(action_id) {
            return;
        }
        self.outstanding_arm = None;
        if accepted {
            self.pending_events
                .push(MissionEvent::ArmAccepted { action_id });
            if self.config.cruise_height_m > 0.0 {
                self.state = MissionState::Climb;
                self.pending_events.push(MissionEvent::ClimbStarted);
            } else {
                self.state = MissionState::Enroute;
                self.pending_events.push(MissionEvent::EnrouteStarted);
            }
        } else {
            self.counters.arm_rejected = self.counters.arm_rejected.wrapping_add(1);
            self.arm_needs_send = true;
            self.pending_events
                .push(MissionEvent::ArmRejected { action_id });
        }
    }

    /// The named refusal/rejection counters.
    #[must_use]
    pub const fn counters(&self) -> &MissionCounters {
        &self.counters
    }

    /// The current mission phase.
    #[must_use]
    pub const fn state(&self) -> MissionState {
        self.state
    }
}

/// Converts expanded route points into plan waypoints: degrees to
/// radians once, cruise altitude as an `At` constraint everywhere.
fn build_waypoints(
    points: &[aerocontext_planning::route::RoutePoint],
    config: &MissionConfig,
) -> Vec<Waypoint> {
    let cruise_alt_m = config.anchor.altitude_m + config.cruise_height_m;
    points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let ident = point.ident.clone().unwrap_or_else(|| format!("RP{index}"));
            let position = GeodeticPosition::new(
                point.position.lat.to_radians(),
                point.position.lon.to_radians(),
                cruise_alt_m,
            );
            Waypoint::new(ident, position).with_altitude(AltitudeConstraint::At(cruise_alt_m))
        })
        .collect()
}

/// Guidance shaped by the mission config: cruise speed and ceilings are
/// the mission's; the lateral/vertical gains and the arrival slowdown
/// keep the upstream defaults.
fn guidance_config(config: &MissionConfig) -> VelocityGuidanceConfig {
    let mut guidance = VelocityGuidanceConfig::default();
    guidance.cruise_mps = config.cruise_mps;
    guidance.max_horizontal_mps = config.limits.max_horizontal_mps;
    guidance.max_vertical_mps = config.limits.max_vertical_mps;
    guidance
}
