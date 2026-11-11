//! The way home as a plan of its own: one fly-over waypoint over the
//! anchor at the cruise height. A lane flies it when a successor ends the
//! work after the task in progress.

use navigate_contract::{AltitudeConstraint, Ident, PlanRole, TurnType, Waypoint};
use navigate_geodesy::NedOffset;
use navigate_route::ExecutionConfig;
use pilotage_mission_core::{Digest, NavigationDataIdentity};
use sha2::{Digest as _, Sha256};

use super::{MissionEngine, document};
use crate::config::MissionConfig;
use crate::error::MissionBuildError;
use crate::route::MissionRoute;

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
        let plane = crate::body::tangent_plane(config.anchor)?;
        let mut position = plane.from_ned(&NedOffset::new(0.0, 0.0, -config.cruise_height_m));
        let altitude_m = config.anchor.altitude_m + config.cruise_height_m;
        position.altitude_m = altitude_m;
        let home = Waypoint::new(home_ident()?, position)
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
        let plan_id = format!("home:{digest}");
        let route = MissionRoute::activate(
            "home",
            PlanRole::Mission,
            vec![home],
            ExecutionConfig::new(HOME_CAPTURE_M),
        )?;
        let segments = document::home_segments(route.waypoints().len(), &config.task_ids);
        Self::from_route(
            route,
            &plan_id,
            config,
            identity,
            document::HOME_REVISION_PREFIX,
            &segments,
        )
    }
}

fn home_ident() -> Result<Ident, MissionBuildError> {
    Ident::try_new("HOME").map_err(|source| MissionBuildError::NavigationIdent {
        ident: "HOME".to_owned(),
        source,
    })
}
