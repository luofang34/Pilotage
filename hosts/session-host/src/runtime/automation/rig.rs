//! In-process rigs over the engine actor and the mission principal, and
//! the synthetic clients a test drives them with.

use pilotage_session::ClientKey;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::{AutomationStatus, OBSERVER_CLIENT};
#[cfg(feature = "sim")]
use super::{prepare, spawn_mission_task};
#[cfg(feature = "sim")]
use crate::error::HostError;
#[cfg(feature = "sim")]
use crate::runtime::RuntimeOptions;
use crate::runtime::connection::ToConnection;
use crate::runtime::engine_actor::ToEngine;
#[cfg(feature = "sim")]
use crate::runtime::engine_actor::{ENGINE_QUEUE_CAPACITY, EngineActor};
use crate::runtime::registry::OUTBOUND_QUEUE_CAPACITY;
#[cfg(feature = "sim")]
use crate::runtime::{MissionOptions, MissionPlanRequest};
#[cfg(feature = "sim")]
use pilotage_mission::SurveyRequest;

/// An in-process test rig: the reference engine actor plus the mission
/// principal, wired exactly as the host runtime wires them for
/// [`crate::cli::AdapterKind::Reference`] — no transport endpoint.
pub struct MissionRig {
    pub(super) engine_tx: mpsc::Sender<ToEngine>,
    pub(super) status: watch::Receiver<AutomationStatus>,
    pub(super) actor: JoinHandle<()>,
    pub(super) automation: JoinHandle<()>,
    pub(super) start: Instant,
}

/// A synthetic operator client registered with the engine actor: it
/// sends envelopes as a remote connection would and reads the replies and
/// authority events the actor sends it.
pub struct Participant {
    pub(super) client: ClientKey,
    pub(super) engine_tx: mpsc::Sender<ToEngine>,
    pub(super) outbound: mpsc::Receiver<ToConnection>,
    pub(super) start: Instant,
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
    pub(super) outbound: mpsc::Receiver<ToConnection>,
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
    let (engine, adapter) = crate::runtime::build_reference(RuntimeOptions::default());
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
