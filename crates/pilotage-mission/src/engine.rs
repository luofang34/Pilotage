//! The sans-IO mission engine: snapshot + route in, typed intents and
//! actions out. All time is caller-supplied; nothing here reads a clock
//! or performs I/O.

mod document;
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
use pilotage_mission_core::{
    ActionId, DirectiveReceipt, EngineState as CoreEngineState, FlightAction as CoreFlightAction,
    FlightPlanReference, MissionDocument, MissionEngine as CoreMissionEngine,
    MissionTerminal as CoreMissionTerminal, PhaseStage, ReceiptResult,
};

pub use nav_guidance::{NavGuidance, NavQuality};
pub use output::{MissionAction, MissionCounters, MissionEvent, MissionOutput, MissionState};

use crate::config::MissionConfig;
use crate::error::MissionBuildError;
use crate::ownship::{OwnshipSample, TruthRole};
use crate::provenance::{MissionPlanRecord, SnapshotProvenance};

/// The single synthesized-GNSS source identity the engine stamps its
/// observations with.
const OWNSHIP_SOURCE: SourceId = SourceId::new(1);

/// A Navigate-backed operational handler around the shared mission core.
///
/// The host owns the boundary: it converts telemetry into
/// [`OwnshipSample`]s, frames the emitted intents and actions onto the
/// session wire, and supplies each `now` on the configured clock domain.
/// The shared core owns phase transitions. This handler owns Navigate
/// observations, guidance, and directive interpretation.
#[derive(Debug)]
pub struct MissionEngine {
    config: MissionConfig,
    plane: LocalTangentPlane,
    filter: NavigationFilter,
    execution: PlanExecution,
    guidance: VelocityGuidanceConfig,
    document: MissionDocument,
    plan_reference: FlightPlanReference,
    core: Option<CoreMissionEngine>,
    core_failed: bool,
    active_action: Option<CoreFlightAction>,
    pending_receipt: Option<DirectiveReceipt>,
    outstanding_arm: Option<ActionId>,
    plan_complete: bool,
    counters: MissionCounters,
    /// The solution the last tick published, backing the display-facing
    /// guidance view. A tick whose filter published nothing clears it, so
    /// [`MissionEngine::nav_guidance`] cannot serve stale geometry.
    last_solution: Option<NavigationSolution>,
    /// The latest known heading. `None` until a sample carries an
    /// attitude group: intents need the NED→body rotation, and guessing
    /// zero would silently rotate every command to due north.
    last_yaw_rad: Option<f64>,
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
    /// the sequencer refuses to activate.
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
        let plan = FlightPlan::new(
            format!("mission:{}", config.route),
            PlanRole::Mission,
            waypoints,
        );
        let execution = PlanExecution::new(plan, ExecutionConfig::default())?;
        let (document, plan_reference) = document::build_document(
            execution.plan(),
            &config,
            provenance.navigation_data_identity.clone(),
        )?;
        let record = MissionPlanRecord {
            provenance,
            route_input: config.route.clone(),
            waypoint_count: expanded_idents.len(),
            expanded_idents,
            mission_identity: document.identity.clone(),
        };
        let plane = LocalTangentPlane::new(config.anchor)?;
        let filter = NavigationFilter::new(FusionConfig::default(), config.clock);
        let guidance = guidance_config(&config);
        let engine = Self {
            config,
            plane,
            filter,
            execution,
            guidance,
            document,
            plan_reference,
            core: None,
            core_failed: false,
            active_action: None,
            pending_receipt: None,
            outstanding_arm: None,
            plan_complete: false,
            counters: MissionCounters::default(),
            last_solution: None,
            last_yaw_rad: None,
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
    /// Acceptance completes the correlated core directive. Rejection is
    /// a retryable receipt, so the core emits a fresh identifier on the
    /// next tick. Results for unknown identifiers are inert.
    pub fn on_action_result(&mut self, action_id: u64, accepted: bool) {
        let Some(outstanding) = self.outstanding_arm else {
            return;
        };
        if u64::from(outstanding.get()) != action_id {
            return;
        }
        self.outstanding_arm = None;
        let result = if accepted {
            ReceiptResult::Succeeded {}
        } else {
            self.counters.arm_rejected = self.counters.arm_rejected.wrapping_add(1);
            ReceiptResult::Retryable {
                detail: "the vehicle rejected the arm action".to_owned(),
            }
        };
        self.pending_receipt = Some(DirectiveReceipt {
            action_id: outstanding,
            result,
        });
    }

    /// The named refusal/rejection counters.
    #[must_use]
    pub const fn counters(&self) -> &MissionCounters {
        &self.counters
    }

    /// The current mission phase.
    #[must_use]
    pub fn state(&self) -> MissionState {
        if self.core_failed {
            return MissionState::Failed;
        }
        let Some(core) = self.core.as_ref() else {
            return MissionState::AwaitSolution;
        };
        match core.state() {
            CoreEngineState::Running {
                phase_id, stage, ..
            } if phase_id == document::ARM_PHASE_ID => match stage {
                PhaseStage::WaitingForEntry {} => MissionState::AwaitSolution,
                PhaseStage::WaitingForReceipt { .. } | PhaseStage::WaitingForCompletion {} => {
                    MissionState::Arming
                }
            },
            CoreEngineState::Running { phase_id, .. } if phase_id == document::CLIMB_PHASE_ID => {
                MissionState::Climb
            }
            CoreEngineState::Running { phase_id, .. }
                if phase_id == document::FOLLOW_PLAN_PHASE_ID =>
            {
                MissionState::Enroute
            }
            CoreEngineState::Terminal {
                result: CoreMissionTerminal::Complete { .. },
            } => MissionState::Complete,
            CoreEngineState::CleaningUp { .. }
            | CoreEngineState::Running { .. }
            | CoreEngineState::Terminal { .. } => MissionState::Failed,
        }
    }

    /// Gets the validated mission document that owns phase transitions.
    #[must_use]
    pub const fn mission_document(&self) -> &MissionDocument {
        &self.document
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
