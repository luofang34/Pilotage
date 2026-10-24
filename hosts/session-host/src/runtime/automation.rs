//! The in-process mission principal (ADR-0025, ADR-0030): a local
//! automation client that speaks the session vocabulary verbatim —
//! hello, profile activation, lease, typed intent frames, reliable
//! action commands — against the same engine actor remote operators
//! reach over WebTransport, so it is fenced by exactly the same
//! authority rules.
//!
//! The task holds only a [`tokio::sync::mpsc::WeakSender`] to the engine
//! actor: it can never keep the actor's command channel open on its own,
//! so host shutdown proceeds exactly as without a mission and the task
//! drains out behind the actor.

pub(crate) mod ownship;
mod rig;
#[cfg(feature = "sim")]
mod sim_multicopter_rig;
mod task;

pub use rig::{MissionRig, Participant, TelemetryObserver};
#[cfg(feature = "sim")]
pub use rig::{spawn_reference_mission_rig, spawn_reference_survey_rig};
pub(crate) use task::RunControl;

use navigate_contract::{ClockDomainId, GeodeticPosition};
use pilotage_mission::{MissionConfig, MissionEngine, MissionState, SurveyRequest};
use pilotage_session::ClientKey;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::info;

use crate::error::HostError;
use crate::mission_navdata::{self, LoadedNavdata};
use crate::runtime::engine_actor::ToEngine;
use crate::runtime::registry::OUTBOUND_QUEUE_CAPACITY;
use crate::runtime::{MissionOptions, MissionPlanRequest};
#[cfg(feature = "sim")]
pub use sim_multicopter_rig::{SurveyRuns, spawn_sim_multicopter_rig, spawn_sim_multicopter_runs};

/// The mission principal's driver-assigned client key. The accept loop
/// allocates keys from zero with a `fetch_add`, so `u64::MAX` can only
/// collide after 2^64 - 1 accepted connections — beyond any process
/// lifetime — making it collision-free for the local principal.
const MISSION_CLIENT: ClientKey = ClientKey::new(u64::MAX);

/// The observing client's key, allocated from the same collision-free end
/// of the space as [`MISSION_CLIENT`].
const OBSERVER_CLIENT: ClientKey = ClientKey::new(u64::MAX - 1);

/// The single clock domain every mission `now`, ownship stamp, and fusion
/// judgment lives on: the host's shared monotonic origin (ADR-0009).
/// The climb rate a lane's plans ask for, metres per second.
const LANE_CLIMB_RATE_MPS: f64 = 3.0;

const MISSION_CLOCK: ClockDomainId = ClockDomainId::new(1);

/// Observable progress of the in-process mission principal, published on
/// a watch channel for logs and the integration test.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AutomationStatus {
    /// Session id assigned by the welcome, once the handshake completed.
    pub session: Option<u64>,
    /// The control-profile activation announcement has been sent.
    pub activation_sent: bool,
    /// Fencing generation of the held motion lease.
    pub lease_generation: Option<u64>,
    /// The engine closed this principal's connection, or the mission is
    /// otherwise ended for this task; nothing resumes it.
    pub fenced: bool,
    /// A human holds the motion scope: the mission stopped framing and
    /// waits for a transfer back (ADR-0051).
    pub held: bool,
    /// A real vehicle's mission is waiting for its pilot to launch it.
    pub awaiting_launch: bool,
    /// A real vehicle's mission is waiting for the vehicle to advertise
    /// velocity control.
    pub awaiting_capabilities: bool,
    /// Lease requests scheduled while another principal held the scope.
    pub lease_retries: u32,
    /// Transfers back that resumed the mission.
    pub resumes: u32,
    /// Offers this principal made to humans who requested the scope.
    pub offers: u32,
    /// The vehicle accepted the arm action.
    pub arm_accepted: bool,
    /// Current mission phase, once the engine is built.
    pub mission_state: Option<MissionState>,
    /// Typed intent frames submitted to the session engine.
    pub frames_sent: u64,
    /// `FrameRejected` notices received back for submitted frames.
    pub frames_rejected: u64,
    /// The engine closed this principal's connection.
    pub closed: bool,
    /// Index of the waypoint the plan flies toward, once the engine is
    /// built.
    pub target_waypoint: Option<usize>,
    /// The plan's last waypoint is captured.
    pub plan_complete: bool,
    /// An operator's hold keeps the vehicle stopped in place.
    pub operator_held: bool,
    /// An abort stopped the vehicle, released the lease and ended the run.
    pub stopped: bool,
    /// The vehicle disarmed or landed on its own during a flown phase, and
    /// the run ended for that reason.
    pub ended_by_vehicle: Option<&'static str>,
    /// An operator's hold or an abort holds the vehicle, and the vehicle
    /// reports that it is still.
    pub holding: bool,
}

/// Where the plan comes from: a navdata snapshot the route expands
/// against, or an area survey request.
pub(crate) enum PlanSource {
    /// The decoded snapshot and its provenance.
    Navdata(LoadedNavdata),
    /// The survey request the plan is derived from.
    Survey(SurveyRequest),
    /// Home from wherever the vehicle is, at the cruise height.
    Home,
}

/// A validated mission: the plan source and the config the flight engine
/// is built from once capabilities are known.
pub(crate) struct MissionPlan {
    source: PlanSource,
    config: MissionConfig,
}

impl MissionPlan {
    /// The mission config over this plan's route and anchor; limits are
    /// tightened by the caller from the advertised capabilities.
    fn config(&self) -> MissionConfig {
        self.config.clone()
    }

    /// The plan of a run a mission lane admitted, over its config.
    pub(crate) fn of_lane(plan: pilotage_mission::RunPlan, config: MissionConfig) -> Self {
        let source = match plan {
            pilotage_mission::RunPlan::Survey(request) => PlanSource::Survey(request),
            pilotage_mission::RunPlan::Home => PlanSource::Home,
        };
        Self { source, config }
    }

    /// The config every lane proposal starts from; compilation sets the
    /// anchor, height and speed from the objective.
    pub(crate) fn lane_template(
        ownship_source: pilotage_mission::OwnshipSource,
        execution_target: pilotage_mission::ExecutionTarget,
    ) -> MissionConfig {
        let mut config = MissionConfig::new(
            String::new(),
            GeodeticPosition::new(0.0, 0.0, 0.0),
            MISSION_CLOCK,
        );
        config.ownship_source = ownship_source;
        config.execution_target = execution_target;
        // A lane's plans climb to a survey height of a hundred metres or
        // more; the vehicle's advertised envelope still tightens both.
        config.climb_rate_mps = LANE_CLIMB_RATE_MPS;
        config.limits.max_vertical_mps = LANE_CLIMB_RATE_MPS;
        config
    }

    /// The mission config the options describe. A survey carries no route.
    fn config_for(options: &MissionOptions) -> MissionConfig {
        let route = match &options.plan {
            MissionPlanRequest::Route(route) => route.clone(),
            MissionPlanRequest::Survey(_) => String::new(),
        };
        // The anchor in radians/meters, converted exactly once (ADR-0030).
        let anchor = GeodeticPosition::new(
            options.anchor.lat_deg.to_radians(),
            options.anchor.lon_deg.to_radians(),
            options.anchor.alt_m,
        );
        let mut config = MissionConfig::new(route, anchor, MISSION_CLOCK);
        if let Some(cruise_height_m) = options.cruise_height_m {
            config.cruise_height_m = cruise_height_m;
        }
        config.ownship_source = options.ownship_source;
        config.execution_target = options.execution_target;
        if let Some(cruise_mps) = options.cruise_mps {
            // The ceiling follows the asked speed here; the welcome tightens
            // it to the advertised envelope before the engine flies.
            config.cruise_mps = cruise_mps;
            config.limits.max_horizontal_mps = config.limits.max_horizontal_mps.max(cruise_mps);
        }
        config
    }
}

/// Proves the plan builds and logs its record — all at startup, so a bad
/// route, store, or area fails the host before it listens. A route loads
/// the navdata and logs the ADR-0030 pack-for-flight record; a survey
/// logs the lines it plans.
///
/// # Errors
///
/// [`HostError::MissionNavdata`] when the snapshot cannot be loaded;
/// [`HostError::MissionBuild`] when the plan does not build.
pub(crate) fn prepare(options: &MissionOptions) -> Result<MissionPlan, HostError> {
    match &options.plan {
        MissionPlanRequest::Route(_) => prepare_route(options),
        MissionPlanRequest::Survey(request) => prepare_survey(options, request),
    }
}

fn prepare_survey(
    options: &MissionOptions,
    request: &SurveyRequest,
) -> Result<MissionPlan, HostError> {
    let plan = MissionPlan {
        source: PlanSource::Survey(request.clone()),
        config: MissionPlan::config_for(options),
    };
    let (_, record) = MissionEngine::from_survey(request, plan.config())
        .map_err(|source| HostError::MissionBuild(Box::new(source)))?;
    info!(
        vertices = request.polygon_deg.len(),
        lines = record.line_count,
        waypoints = record.waypoint_count,
        line_spacing_m = record.line_spacing_m,
        footprint_width_m = record.footprint_width_m,
        path_length_m = record.path_length_m,
        survey_digest = %record.digest,
        mission_revision = %record.mission_identity.revision_id,
        mission_digest = %record.mission_identity.content_digest,
        "survey planned for flight"
    );
    Ok(plan)
}

fn prepare_route(options: &MissionOptions) -> Result<MissionPlan, HostError> {
    let navdata = mission_navdata::load(options).map_err(HostError::MissionNavdata)?;
    let (_, record) = MissionEngine::new(
        &navdata.snapshot,
        navdata.provenance.clone(),
        MissionPlan::config_for(options),
    )
    .map_err(|source| HostError::MissionBuild(Box::new(source)))?;
    let plan = MissionPlan {
        source: PlanSource::Navdata(navdata),
        config: MissionPlan::config_for(options),
    };
    info!(
        route = %record.route_input,
        waypoints = record.waypoint_count,
        expanded = ?record.expanded_idents,
        authority = %record.provenance.authority,
        effective_on = %record.provenance.effective_on,
        sha256 = %record.provenance.sha256_hex,
        fixture = record.provenance.fixture,
        mission_revision = %record.mission_identity.revision_id,
        mission_schema = record.mission_identity.schema_version,
        mission_digest = %record.mission_identity.content_digest,
        navdata_cycle = %record.mission_identity.navigation_data_identity.cycle,
        navdata_snapshot = %record.mission_identity.navigation_data_identity.snapshot_id,
        navdata_digest = %record.mission_identity.navigation_data_identity.snapshot_digest,
        "mission packed for flight"
    );
    Ok(plan)
}

/// Spawns the mission principal against `engine_tx`'s actor, sharing the
/// host's monotonic origin `start`. `planar_pose_is_truth` declares
/// whether this host's unstamped planar poses are the simulator's own
/// state — true only for the deterministic reference adapter. Returns
/// the task handle and the status watch.
pub(crate) fn spawn_mission_task(
    engine_tx: &mpsc::Sender<ToEngine>,
    start: Instant,
    plan: MissionPlan,
    planar_pose_is_truth: bool,
) -> (JoinHandle<()>, watch::Receiver<AutomationStatus>) {
    spawn_mission_run(
        engine_tx,
        start,
        plan,
        planar_pose_is_truth,
        MISSION_CLIENT,
        None,
    )
}

/// Spawns one run of a mission lane as its own principal under `client`,
/// steered by `controls`.
pub(crate) fn spawn_mission_run(
    engine_tx: &mpsc::Sender<ToEngine>,
    start: Instant,
    plan: MissionPlan,
    planar_pose_is_truth: bool,
    client: ClientKey,
    controls: Option<mpsc::Receiver<RunControl>>,
) -> (JoinHandle<()>, watch::Receiver<AutomationStatus>) {
    let (status_tx, status_rx) = watch::channel(AutomationStatus::default());
    let (outbound_tx, outbound_rx) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
    let mut task = task::MissionTask::new(
        engine_tx.downgrade(),
        start,
        plan,
        status_tx,
        planar_pose_is_truth,
    );
    task.bind(client, controls);
    let handle = tokio::spawn(async move {
        task.run(outbound_tx, outbound_rx).await;
        tracing::debug!(task = "mission-automation", "task exited");
    });
    (handle, status_rx)
}
