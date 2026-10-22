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

mod ownship;
mod task;

use navigate_contract::{ClockDomainId, GeodeticPosition};
use pilotage_mission::{MissionConfig, MissionEngine, MissionState, SurveyRequest};
use pilotage_session::ClientKey;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::info;

use crate::error::HostError;
use crate::mission_navdata::{self, LoadedNavdata};
#[cfg(feature = "sim")]
use crate::runtime::RuntimeOptions;
use crate::runtime::connection::ToConnection;
use crate::runtime::engine_actor::ToEngine;
#[cfg(feature = "sim")]
use crate::runtime::engine_actor::{ENGINE_QUEUE_CAPACITY, EngineActor};
use crate::runtime::registry::OUTBOUND_QUEUE_CAPACITY;
use crate::runtime::{MissionOptions, MissionPlanRequest};

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
}

/// Where the plan comes from: a navdata snapshot the route expands
/// against, or an area survey request.
pub(crate) enum PlanSource {
    /// The decoded snapshot and its provenance.
    Navdata(LoadedNavdata),
    /// The survey request the plan is derived from.
    Survey(SurveyRequest),
}

/// A validated mission: the plan source and the options the flight engine
/// is built from once capabilities are known.
pub(crate) struct MissionPlan {
    source: PlanSource,
    options: MissionOptions,
}

impl MissionPlan {
    /// The mission config over this plan's route and anchor; limits are
    /// tightened by the caller from the advertised capabilities.
    fn config(&self) -> MissionConfig {
        Self::config_for(&self.options)
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
        options: options.clone(),
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
        options: options.clone(),
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
    let (status_tx, status_rx) = watch::channel(AutomationStatus::default());
    let (outbound_tx, outbound_rx) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
    let task = task::MissionTask::new(
        engine_tx.downgrade(),
        start,
        plan,
        status_tx,
        planar_pose_is_truth,
    );
    let handle = tokio::spawn(async move {
        task.run(outbound_tx, outbound_rx).await;
        tracing::debug!(task = "mission-automation", "task exited");
    });
    (handle, status_rx)
}

/// An in-process test rig: the reference engine actor plus the mission
/// principal, wired exactly as the host runtime wires them for
/// [`crate::cli::AdapterKind::Reference`] — no transport endpoint.
pub struct MissionRig {
    engine_tx: mpsc::Sender<ToEngine>,
    status: watch::Receiver<AutomationStatus>,
    actor: JoinHandle<()>,
    automation: JoinHandle<()>,
    start: Instant,
}

/// A synthetic operator client registered with the engine actor: it
/// sends envelopes as a remote connection would and reads the replies and
/// authority events the actor sends it.
pub struct Participant {
    client: ClientKey,
    engine_tx: mpsc::Sender<ToEngine>,
    outbound: mpsc::Receiver<ToConnection>,
    start: Instant,
}

impl Participant {
    /// Sends one envelope as this client, stamped with the host clock.
    pub async fn send(&self, message: pilotage_session::DomainEnvelope) -> bool {
        let nanos = u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.engine_tx
            .send(ToEngine::ClientMessage {
                client: self.client,
                message: Box::new(message),
                now: pilotage_timing::MonoTimestamp::from_nanos(nanos),
            })
            .await
            .is_ok()
    }

    /// The next reliable bootstrap reply, or `None` once the actor is gone.
    pub async fn next_bootstrap(&mut self) -> Option<Vec<u8>> {
        while let Some(message) = self.outbound.recv().await {
            if let ToConnection::BootstrapMessage { bytes, .. } = message {
                return Some(bytes);
            }
        }
        None
    }

    /// The next authority event, or `None` once the actor is gone.
    pub async fn next_authority(&mut self) -> Option<Vec<u8>> {
        while let Some(message) = self.outbound.recv().await {
            if let ToConnection::AuthorityMessage(bytes) = message {
                return Some(bytes);
            }
        }
        None
    }
}

/// A synthetic client registered with the engine actor purely to read
/// what it broadcasts: the same outbound path a remote connection's
/// writer half drains, so a test reads exactly the bytes that would go on
/// the wire.
pub struct TelemetryObserver {
    outbound: mpsc::Receiver<ToConnection>,
}

impl TelemetryObserver {
    /// The next best-effort datagram broadcast to this observer, or
    /// `None` once the engine actor is gone.
    pub async fn next_datagram(&mut self) -> Option<Vec<u8>> {
        while let Some(message) = self.outbound.recv().await {
            if let ToConnection::Datagram { bytes, .. } = message {
                return Some(bytes);
            }
        }
        None
    }
}

impl MissionRig {
    /// A fresh handle on the mission principal's status watch.
    #[must_use]
    pub fn status(&self) -> watch::Receiver<AutomationStatus> {
        self.status.clone()
    }

    /// Registers an observing client with the engine actor and hands back
    /// its outbound datagrams. Dropping the observer closes the channel
    /// and the actor evicts it on the next send.
    pub async fn observe(&self) -> TelemetryObserver {
        let (outbound_tx, outbound) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
        self.engine_tx
            .send(ToEngine::ClientConnected {
                client: OBSERVER_CLIENT,
                sender: outbound_tx,
            })
            .await
            .ok();
        TelemetryObserver { outbound }
    }

    /// Registers an operator client under `client`, a key the accept loop
    /// never allocates in a test, and hands back its two-way handle.
    pub async fn participant(&self, client: u64) -> Participant {
        let (outbound_tx, outbound) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
        let client = ClientKey::new(client);
        self.engine_tx
            .send(ToEngine::ClientConnected {
                client,
                sender: outbound_tx,
            })
            .await
            .ok();
        Participant {
            client,
            engine_tx: self.engine_tx.clone(),
            outbound,
            start: self.start,
        }
    }

    /// Closes the engine actor's command channel and waits for the actor
    /// and the mission principal to drain out, proving the shutdown
    /// ordering the host relies on.
    pub async fn shutdown(self) {
        drop(self.engine_tx);
        self.actor.await.ok();
        self.automation.await.ok();
    }
}

/// Spawns [`MissionRig`] over the fixture mission with a zero cruise
/// height (the planar reference vehicle has no climb axis).
///
/// # Errors
///
/// [`HostError`] when the fixture snapshot or demo route fails to build.
#[cfg(feature = "sim")]
pub fn spawn_reference_mission_rig() -> Result<MissionRig, HostError> {
    let options = MissionOptions {
        plan: MissionPlanRequest::Route(pilotage_mission::fixture::DEMO_ROUTE.to_owned()),
        navdata: crate::runtime::MissionNavdataSource::Fixture,
        anchor: crate::runtime::DEFAULT_MISSION_ANCHOR,
        date: None,
        cruise_height_m: Some(0.0),
        ownship_source: pilotage_mission::OwnshipSource::SimulationTruth,
        execution_target: pilotage_mission::ExecutionTarget::Simulator,
        cruise_mps: None,
    };
    spawn_reference_rig(options)
}

/// Spawns [`MissionRig`] over a small area survey at a zero height with
/// a measured footprint (the planar reference vehicle has no climb axis):
/// a 50 m by 60 m rectangle north-east of the default anchor, two lines
/// 20 m apart, which the car-like reference vehicle can turn between.
///
/// # Errors
///
/// [`HostError`] when the survey does not plan.
#[cfg(feature = "sim")]
pub fn spawn_reference_survey_rig() -> Result<MissionRig, HostError> {
    let anchor = crate::runtime::DEFAULT_MISSION_ANCHOR;
    // About 50 m north and 60 m east at the anchor's latitude.
    let north = 50.0 / 111_320.0;
    let east = 60.0 / (111_320.0 * anchor.lat_deg.to_radians().cos());
    let mut request = SurveyRequest::new(vec![
        [anchor.lat_deg, anchor.lon_deg],
        [anchor.lat_deg + north, anchor.lon_deg],
        [anchor.lat_deg + north, anchor.lon_deg + east],
        [anchor.lat_deg, anchor.lon_deg + east],
    ]);
    request.footprint_width_m = Some(40.0);
    // A 24 m step covers the 20 m beyond the first swath in one more
    // line: exactly two lines across the 60 m width.
    request.sidelap = 0.4;
    request.overshoot_m = 5.0;
    request.line_bearing_rad = Some(0.0);
    let options = MissionOptions {
        plan: MissionPlanRequest::Survey(request),
        navdata: crate::runtime::MissionNavdataSource::Fixture,
        anchor,
        date: None,
        cruise_height_m: Some(0.0),
        ownship_source: pilotage_mission::OwnshipSource::SimulationTruth,
        execution_target: pilotage_mission::ExecutionTarget::Simulator,
        cruise_mps: None,
    };
    spawn_reference_rig(options)
}

#[cfg(feature = "sim")]
fn spawn_reference_rig(options: MissionOptions) -> Result<MissionRig, HostError> {
    let plan = prepare(&options)?;
    let start = Instant::now();
    let (engine, adapter) = super::build_reference(RuntimeOptions::default());
    let (engine_tx, engine_rx) = mpsc::channel(ENGINE_QUEUE_CAPACITY);
    let actor = tokio::spawn(EngineActor::new(engine, adapter, start).run(engine_rx));
    let (automation, status) = spawn_mission_task(&engine_tx, start, plan, true);
    Ok(MissionRig {
        engine_tx,
        status,
        actor,
        start,
        automation,
    })
}
