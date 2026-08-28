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
use pilotage_mission::{MissionConfig, MissionEngine, MissionState};
use pilotage_session::ClientKey;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::info;

use crate::error::HostError;
use crate::mission_navdata::{self, LoadedNavdata};
use crate::runtime::MissionOptions;
#[cfg(feature = "sim")]
use crate::runtime::RuntimeOptions;
use crate::runtime::connection::ToConnection;
use crate::runtime::engine_actor::ToEngine;
#[cfg(feature = "sim")]
use crate::runtime::engine_actor::{ENGINE_QUEUE_CAPACITY, EngineActor};
use crate::runtime::registry::OUTBOUND_QUEUE_CAPACITY;

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
    /// Authority moved away (denied, revoked, or granted to another
    /// principal): the mission stopped framing and holds — it never
    /// re-leases on its own, human takeover wins (ADR-0025).
    pub fenced: bool,
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

/// A validated mission: the decoded snapshot, its provenance, and the
/// options the flight engine is built from once capabilities are known.
pub(crate) struct MissionPlan {
    navdata: LoadedNavdata,
    options: MissionOptions,
}

impl MissionPlan {
    /// The anchor in radians/meters, converted exactly once (ADR-0030).
    fn anchor(&self) -> GeodeticPosition {
        GeodeticPosition::new(
            self.options.anchor.lat_deg.to_radians(),
            self.options.anchor.lon_deg.to_radians(),
            self.options.anchor.alt_m,
        )
    }

    /// The mission config over this plan's route and anchor; limits are
    /// tightened by the caller from the advertised capabilities.
    fn config(&self) -> MissionConfig {
        let mut config =
            MissionConfig::new(self.options.route.clone(), self.anchor(), MISSION_CLOCK);
        if let Some(cruise_height_m) = self.options.cruise_height_m {
            config.cruise_height_m = cruise_height_m;
        }
        config
    }
}

/// Loads the navdata, proves the route builds against it, and logs the
/// ADR-0030 pack-for-flight record — all at startup, so a bad route or
/// store fails the host before it listens.
///
/// # Errors
///
/// [`HostError::MissionNavdata`] when the snapshot cannot be loaded;
/// [`HostError::MissionBuild`] when the route does not build against it.
pub(crate) fn prepare(options: &MissionOptions) -> Result<MissionPlan, HostError> {
    let navdata = mission_navdata::load(options).map_err(HostError::MissionNavdata)?;
    let plan = MissionPlan {
        navdata,
        options: options.clone(),
    };
    let (_, record) = MissionEngine::new(
        &plan.navdata.snapshot,
        plan.navdata.provenance.clone(),
        plan.config(),
    )
    .map_err(|source| HostError::MissionBuild(Box::new(source)))?;
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
        route: pilotage_mission::fixture::DEMO_ROUTE.to_owned(),
        navdata: crate::runtime::MissionNavdataSource::Fixture,
        anchor: crate::runtime::DEFAULT_MISSION_ANCHOR,
        date: None,
        cruise_height_m: Some(0.0),
    };
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
        automation,
    })
}
