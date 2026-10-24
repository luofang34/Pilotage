//! A task with no plan, for tests of behaviour independent of mission
//! content.

use std::collections::HashMap;

use pilotage_mission::OwnshipSource;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use super::{AutomationStatus, MISSION_CLIENT, MissionTask};
use crate::runtime::engine_actor::ToEngine;

impl MissionTask {
    /// A task with no plan, for unit tests of session/fencing behavior
    /// that is independent of mission content.
    #[cfg(test)]
    pub(super) fn without_plan(
        engine: mpsc::WeakSender<ToEngine>,
        start: Instant,
        status: watch::Sender<AutomationStatus>,
    ) -> Self {
        Self {
            real_vehicle: false,
            engine,
            start,
            plan: None,
            status,
            session: None,
            principal: None,
            generation: None,
            fenced: false,
            mission: None,
            sequence: 0,
            next_action_id: 0,
            pending_actions: HashMap::new(),
            nav_guidance: None,
            planar_pose_is_truth: true,
            ownship_source: OwnshipSource::SimulationTruth,
            held: false,
            neutral_frames: 0,
            awaiting_launch: false,
            lease_due: false,
            lease_wanted: false,
            lease_retry_at: None,
            client: MISSION_CLIENT,
            controls: None,
            operator_hold: false,
            stopping: None,
            flight: super::flight_watch::FlightWatch::default(),
        }
    }
}
