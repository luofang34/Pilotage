//! The way home as a plan of its own: one fly-over waypoint over the
//! anchor at the cruise height. A lane flies it when a successor ends the
//! work after the task in progress.

use navigate_contract::{AltitudeConstraint, FlightPlan, PlanRole, TurnType, Waypoint};
use navigate_fpl::{ExecutionConfig, PlanExecution};
use navigate_geodesy::{LocalTangentPlane, NedOffset};
use pilotage_mission_core::{Digest, NavigationDataIdentity};
use sha2::{Digest as _, Sha256};

use super::{MissionEngine, document};
use crate::config::MissionConfig;
use crate::error::MissionBuildError;

const HOME_DIGEST_DOMAIN: &[u8] = b"pilotage.home-plan.v1\0";
/// The distance to home at which the plan completes, metres.
const HOME_CAPTURE_M: f64 = 5.0;

impl MissionEngine {
    /// Builds an engine that flies from wherever the vehicle is to the
    /// anchor at the cruise height, and completes there.
    ///
    /// # Errors
    ///
    /// The geodesy, activation and document errors of a survey engine.
    pub fn home(config: MissionConfig) -> Result<Self, MissionBuildError> {
        let plane = LocalTangentPlane::new(config.anchor)?;
        let mut position = plane.from_ned(&NedOffset::new(0.0, 0.0, -config.cruise_height_m));
        let altitude_m = config.anchor.altitude_m + config.cruise_height_m;
        position.altitude_m = altitude_m;
        let home = Waypoint::new("HOME".to_owned(), position)
            .with_altitude(AltitudeConstraint::At(altitude_m))
            .with_turn(TurnType::FlyOver);
        let mut hasher = Sha256::new();
        hasher.update(HOME_DIGEST_DOMAIN);
        for value in [
            config.anchor.latitude_rad,
            config.anchor.longitude_rad,
            altitude_m,
        ] {
            hasher.update(value.to_le_bytes());
        }
        let digest = Digest::from_bytes(hasher.finalize().into());
        let identity = NavigationDataIdentity {
            cycle: "home".to_owned(),
            snapshot_id: format!("home:{digest}"),
            snapshot_digest: digest,
        };
        let plan = FlightPlan::new(format!("home:{digest}"), PlanRole::Mission, vec![home]);
        let execution = PlanExecution::new(plan, ExecutionConfig::new(HOME_CAPTURE_M))?;
        let segments = document::home_segments(execution.plan().waypoints.len(), &config.task_ids);
        Self::from_execution(
            execution,
            config,
            identity,
            document::HOME_REVISION_PREFIX,
            &segments,
        )
    }
}
