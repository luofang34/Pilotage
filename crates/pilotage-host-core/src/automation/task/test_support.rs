//! A task with no plan, for tests of behaviour independent of mission
//! content.

use std::collections::HashMap;

use pilotage_mission::OwnshipSource;

use super::{AutomationStatus, MISSION_CLIENT, MissionTask};

impl MissionTask {
    /// A task with no plan, for unit tests of session/fencing behavior
    /// that is independent of mission content.
    #[cfg(test)]
    pub(super) fn without_plan() -> Self {
        Self {
            real_vehicle: false,
            outbox: Vec::new(),
            now_ns: 0,
            plan: None,
            status: AutomationStatus::default(),
            session: None,
            principal: None,
            generation: None,
            fenced: false,
            mission: None,
            sequence: 0,
            next_action_id: 0,
            pending_actions: HashMap::new(),
            events: Vec::new(),
            nav_guidance: None,
            planar_pose_is_truth: true,
            ownship_source: OwnshipSource::SimulationTruth,
            held: false,
            neutral_frames: 0,
            can_stop: true,
            awaiting_launch: false,
            lease_due: false,
            lease_wanted: false,
            lease_retry_at: None,
            client: MISSION_CLIENT,
            operator_hold: false,
            stopping: None,
            flight: super::flight_watch::FlightWatch::default(),
        }
    }
}
