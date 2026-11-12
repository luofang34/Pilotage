//! The sans-IO mission engine: snapshot + route in, typed intents and
//! actions out. All time is caller-supplied; nothing here reads a clock
//! or performs I/O.

mod dead_reckoning;
mod document;
mod holding_pattern;
mod home;
mod intercept;
mod lateral_mode;
mod lifecycle;
mod modes;
mod nav_guidance;
mod nav_source;
mod navigation_display;
mod onboard;
mod output;
mod ownship;
mod source_integrity;
mod steering;
mod steering_source;
mod tick;
mod track_trim;
mod vehicle_status;
mod vertical_mode;
mod waypoints;

use core::mem::Discriminant;

use aerocontext_core::NavDataSnapshot;
use aerocontext_planning::route::{RouteToken, expand_str};
use navigate_contract::{
    AltitudeConstraint, GeodeticPosition, Ident, MonotonicNanos, NavigationSolution, PlanRole,
    Waypoint,
};
use navigate_fusion::{FusionConfig, NavigationFilter};
use navigate_geodesy::LocalTangentPlane;
use navigate_guidance::{GuidanceRefusal, VelocityGuidanceConfig};
use navigate_route::ExecutionConfig;
use pilotage_mission_core::{
    ActionId, DirectiveReceipt, EngineState as CoreEngineState, FlightAction as CoreFlightAction,
    FlightPlanReference, MissionDocument, MissionEngine as CoreMissionEngine,
    MissionTerminal as CoreMissionTerminal, NavigationDataIdentity, PhaseStage, ReceiptResult,
};

pub use dead_reckoning::DeadReckoning;
pub use lateral_mode::{LateralMode, LateralStatus};
pub use nav_guidance::{NavGuidance, NavQuality};
pub use navigation_display::{
    DisplayFix, DisplayWaypoint, LegDisplay, NavigationDisplay, ToFrom, WaypointProgress,
};
pub use output::{MissionAction, MissionCounters, MissionEvent, MissionOutput, MissionState};
pub use source_integrity::{OnboardSolution, SourceIntegrity, SourceIntegrityState};
pub use steering_source::{
    FlightPhase, PositionIntegrity, PositionKind, PositionSource, SteeringIntegrity, SteeringSource,
};
pub use vehicle_status::{
    LaunchDetection, StartWithheldReason, VEHICLE_STATUS_MAX_AGE_NS, VehicleStatus,
};
pub use vertical_mode::{VerticalMode, VerticalStatus};

use crate::config::MissionConfig;
use crate::error::MissionBuildError;
use crate::route::MissionRoute;

mod estimate;
mod hold;

use crate::provenance::{MissionPlanRecord, SnapshotProvenance};
use crate::survey::{SurveyPlanRecord, SurveyRequest, plan_survey};
use estimate::VehicleEstimate;
use onboard::OnboardNavigation;

/// A Navigate-backed operational handler around the shared mission core.
///
/// The host owns the boundary: it converts telemetry into
/// [`crate::OwnshipSample`]s, frames the emitted intents and actions onto the
/// session wire, and supplies each `now` on the configured clock domain.
/// The shared core owns phase transitions. This handler owns Navigate
/// observations, guidance, and directive interpretation.
#[derive(Debug)]
pub struct MissionEngine {
    config: MissionConfig,
    plane: LocalTangentPlane,
    filter: NavigationFilter,
    route: MissionRoute,
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
    /// The state of the mode machines.
    modes: modes::Modes,
    /// The latest known heading. `None` until a sample carries an
    /// attitude group: intents need the NED→body rotation, and guessing
    /// zero would silently rotate every command to due north.
    last_yaw_rad: Option<f64>,
    last_refusal: Option<Discriminant<GuidanceRefusal>>,
    /// The fix last reported as overflown, so each miss reports once.
    overflown_fix: Option<usize>,
    /// The latest admitted sample under the vehicle-estimate source.
    estimate: VehicleEstimate,
    /// The latest solution the onboard navigator published.
    onboard: OnboardNavigation,
    /// When the current hold began, while a human holds the motion scope.
    hold_started: Option<MonotonicNanos>,
    /// Time spent held before the current hold; the sequencing clock
    /// excludes it.
    held_total_ns: u64,
    /// Hold and resume transitions the next tick announces.
    hold_events: Vec<MissionEvent>,
    /// The vehicle's latest status and the arm request held for it.
    vehicle: vehicle_status::VehicleReadiness,
    /// Whether a simulation-truth sample was ever fused.
    truth_sampled: bool,
    /// The heading and altitude steering of a fixed wing on the onboard
    /// navigator.
    steering: steering::Steering,
    /// The holding pattern flown once the plan is complete, for a vehicle
    /// that cannot stop.
    holding_pattern: Option<holding_pattern::ActivePattern>,
    /// The course trim of the straight track flown, for a vehicle that
    /// cannot stop.
    track_trim: track_trim::TrackTrim,
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
    /// context), a route carrying an unexecuted SID/STAR procedure, an
    /// empty expansion, an implausible anchor, a plan the sequencer
    /// refuses to activate, or the simulation-truth ownship source for a
    /// real vehicle.
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
        if !expanded.procedures.is_empty() {
            return Err(MissionBuildError::UnexecutedProcedure {
                route: config.route.clone(),
                procedures: expanded
                    .procedures
                    .iter()
                    .map(procedure_token_name)
                    .collect(),
            });
        }
        if expanded.points.is_empty() {
            return Err(MissionBuildError::EmptyRoute {
                route: config.route.clone(),
            });
        }
        let waypoints = build_waypoints(&expanded.points, &config)?;
        let expanded_idents: Vec<String> = waypoints
            .iter()
            .map(|wp| wp.ident.as_str().to_owned())
            .collect();
        let plan_id = format!("mission:{}", config.route);
        let name =
            document::navigate_plan_name("mission", &plan_id, PlanRole::Mission, &waypoints)?;
        let route = MissionRoute::activate(
            &name,
            PlanRole::Mission,
            waypoints,
            ExecutionConfig::default(),
        )?;
        let route_input = config.route.clone();
        let engine = Self::from_route(
            route,
            &plan_id,
            config,
            provenance.navigation_data_identity.clone(),
            document::ROUTE_REVISION_PREFIX,
            &[],
        )?;
        let record = MissionPlanRecord {
            provenance,
            route_input,
            waypoint_count: expanded_idents.len(),
            expanded_idents,
            mission_identity: engine.document.identity.clone(),
        };
        Ok((engine, record))
    }

    /// Builds an engine over an area survey of `request` at the configured
    /// cruise height above the anchor. The plan's identity is the survey
    /// request itself: no navigation data takes part.
    ///
    /// # Errors
    ///
    /// [`MissionBuildError::Survey`] when the request does not become a
    /// plan, and the activation, geodesy, and document errors of
    /// [`Self::new`].
    pub fn from_survey(
        request: &SurveyRequest,
        config: MissionConfig,
    ) -> Result<(Self, SurveyPlanRecord), MissionBuildError> {
        let survey = plan_survey(request, &config.anchor, config.cruise_height_m)?;
        let identity = NavigationDataIdentity {
            cycle: "survey".to_owned(),
            snapshot_id: format!("survey:{}", survey.digest),
            snapshot_digest: survey.digest,
        };
        let idents: Vec<String> = survey
            .waypoints
            .iter()
            .map(|waypoint| waypoint.ident.as_str().to_owned())
            .collect();
        let plan_id = format!("survey:{}", survey.digest);
        let name =
            document::navigate_plan_name("survey", &plan_id, PlanRole::Mission, &survey.waypoints)?;
        let route = MissionRoute::activate(
            &name,
            PlanRole::Mission,
            survey.waypoints,
            ExecutionConfig::new(survey.capture_radius_m),
        )?;
        let segments = document::survey_segments(survey.line_count, idents.len(), &config.task_ids);
        let mut engine = Self::from_route(
            route,
            &plan_id,
            config,
            identity,
            document::SURVEY_REVISION_PREFIX,
            &segments,
        )?;
        // A survey line is flown at cruise speed to its end; the arrival
        // slowdown of a route would crawl the last 30 m of every line.
        if engine.guidance.arrival_slowdown_radius_m > 0.0 {
            engine.guidance.arrival_slowdown_radius_m = 2.0 * survey.capture_radius_m;
        }
        let record = SurveyPlanRecord {
            digest: survey.digest,
            line_count: survey.line_count,
            line_spacing_m: survey.line_spacing_m,
            footprint_width_m: survey.footprint_width_m,
            path_length_m: survey.path_length_m,
            capture_radius_m: survey.capture_radius_m,
            waypoint_count: idents.len(),
            idents,
            mission_identity: engine.document.identity.clone(),
        };
        Ok((engine, record))
    }

    /// The engine over an activated plan: the document, the anchor plane,
    /// the fusion filter, and the guidance shaped by the config.
    fn from_route(
        route: MissionRoute,
        plan_id: &str,
        config: MissionConfig,
        identity: NavigationDataIdentity,
        revision_prefix: &str,
        segments: &[document::Segment],
    ) -> Result<Self, MissionBuildError> {
        config.check_ownship_source()?;
        config.check_declared_start()?;
        let (document, plan_reference) = document::build_document(
            plan_id,
            &route.plan(),
            &config,
            identity,
            revision_prefix,
            segments,
        )?;
        let plane = crate::body::tangent_plane(config.anchor)?;
        let filter = NavigationFilter::new(FusionConfig::default(), config.clock);
        let guidance = guidance_config(&config);
        Ok(Self {
            config,
            plane,
            filter,
            route,
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
            modes: modes::Modes::default(),
            last_yaw_rad: None,
            last_refusal: None,
            overflown_fix: None,
            estimate: VehicleEstimate::default(),
            onboard: OnboardNavigation::default(),
            hold_started: None,
            held_total_ns: 0,
            hold_events: Vec::new(),
            vehicle: vehicle_status::VehicleReadiness::default(),
            truth_sampled: false,
            steering: steering::Steering::default(),
            holding_pattern: None,
            track_trim: track_trim::TrackTrim::default(),
        })
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

    /// The operator's declared start, as stated in the configuration, for
    /// the host to hand to the onboard navigator. The engine takes nothing
    /// from it: the navigator's solution is the only ownship it flies.
    #[must_use]
    pub const fn declared_start(&self) -> Option<&crate::DeclaredStart> {
        self.config.declared_start.as_ref()
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
        if self.hold_started.is_some() {
            return MissionState::Held;
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
            CoreEngineState::Running {
                phase_id, stage, ..
            } if phase_id == document::CLIMB_PHASE_ID => match stage {
                PhaseStage::WaitingForEntry {} => MissionState::AwaitSolution,
                PhaseStage::WaitingForReceipt { .. } | PhaseStage::WaitingForCompletion {} => {
                    MissionState::Climb
                }
            },
            // The plan, whole or one of its tasks.
            CoreEngineState::Running { .. } => MissionState::Enroute,
            CoreEngineState::Terminal {
                result: CoreMissionTerminal::Complete { .. },
            } => MissionState::Complete,
            CoreEngineState::CleaningUp { .. } | CoreEngineState::Terminal { .. } => {
                MissionState::Failed
            }
        }
    }

    /// Gets the validated mission document that owns phase transitions.
    #[must_use]
    pub const fn mission_document(&self) -> &MissionDocument {
        &self.document
    }
}

/// Names a recognized-but-unresolved procedure token for the operator,
/// e.g. `"TRUKN2"` or `"BLUES2.IIU"`.
fn procedure_token_name(token: &RouteToken) -> String {
    match token {
        RouteToken::Procedure { name, transition } => match transition {
            Some(transition) => format!("{name}.{transition}"),
            None => name.clone(),
        },
        other => format!("{other:?}"),
    }
}

/// Converts expanded route points into plan waypoints: degrees to
/// radians once, cruise altitude as an `At` constraint everywhere.
fn build_waypoints(
    points: &[aerocontext_planning::route::RoutePoint],
    config: &MissionConfig,
) -> Result<Vec<Waypoint>, MissionBuildError> {
    let cruise_alt_m = config.anchor.altitude_m + config.cruise_height_m;
    points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let name = point.ident.clone().unwrap_or_else(|| format!("RP{index}"));
            let ident =
                Ident::try_new(&name).map_err(|source| MissionBuildError::NavigationIdent {
                    ident: name,
                    source,
                })?;
            let position = GeodeticPosition::new(
                point.position.lat.to_radians(),
                point.position.lon.to_radians(),
                cruise_alt_m,
            );
            Ok(Waypoint::new(ident, position).with_altitude(AltitudeConstraint::At(cruise_alt_m)))
        })
        .collect()
}

/// Guidance shaped by the mission config: cruise speed and ceilings are
/// the mission's; the vertical gain keeps the upstream default.
///
/// A fixed-wing aircraft flies at cruise speed to every fix: it has no
/// arrival slowdown. Its cross-track gain is its rate of turn at the route
/// bank, so the distance it looks ahead to rejoin a track is its turn
/// radius. The upstream gain suits a multirotor that turns on the spot; a
/// fixed wing asked to rejoin over a shorter distance than it can turn
/// crosses the track and swings past it.
fn guidance_config(config: &MissionConfig) -> VelocityGuidanceConfig {
    let mut guidance = VelocityGuidanceConfig::default();
    // A dead-reckoned solution may be graded unusable; the engine has
    // already decided to fly it, so guidance takes it as given.
    if let crate::OwnshipSource::OnboardNavigation {
        beyond_limits: crate::BeyondLimits::DeadReckon { .. },
        ..
    } = config.ownship_source
    {
        guidance.admission.minimum_quality = navigate_contract::SolutionQuality::Unusable;
    }
    guidance.cruise_mps = config.cruise_mps;
    guidance.max_horizontal_mps = config.limits.max_horizontal_mps;
    guidance.max_vertical_mps = config.limits.max_vertical_mps;
    if !config.can_stop() {
        guidance.arrival_slowdown_radius_m = 0.0;
        let cruise_altitude_m = config.anchor.altitude_m + config.cruise_height_m;
        let speed_mps = crate::true_airspeed_for(
            config.cruise_mps.min(config.limits.max_horizontal_mps),
            cruise_altitude_m,
        );
        let radius_m = crate::turn_radius_for(speed_mps, config.route_bank_deg, 0.0);
        if radius_m > 0.0 {
            guidance.cross_track_gain_per_s = speed_mps / radius_m;
        }
    }
    guidance
}
