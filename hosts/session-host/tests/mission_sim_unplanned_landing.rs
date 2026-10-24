//! A run whose vehicle lands on its own: the simulated pack empties during
//! the climb, the adapter lands the vehicle, and the run ends with that
//! reason instead of waiting for a climb that never ends.

#![cfg(feature = "sim")]

use std::time::Duration;

use pilotage_adapter_sim_multicopter::{LaunchPoint, SimConfig, TileSourceConfig};
use pilotage_mission::fixture::GeoPointDegrees;
use pilotage_mission::{ExecutionTarget, MissionState, OwnshipSource, SurveyRequest};
use pilotage_session_host::runtime::{
    self, MissionNavdataSource, MissionOptions, MissionPlanRequest,
};
use tokio::time::timeout;

const LAUNCH: LaunchPoint = LaunchPoint {
    lat_deg: 40.3317,
    lon_deg: -74.6745,
    alt_m: 40.0,
};

fn survey_options() -> MissionOptions {
    let north = 50.0 / 111_320.0;
    let east = 60.0 / (111_320.0 * LAUNCH.lat_deg.to_radians().cos());
    let (lat, lon) = (LAUNCH.lat_deg, LAUNCH.lon_deg);
    let mut request = SurveyRequest::new(vec![
        [lat, lon],
        [lat + north, lon],
        [lat + north, lon + east],
        [lat, lon + east],
    ]);
    request.footprint_width_m = Some(40.0);
    request.line_bearing_rad = Some(0.0);
    MissionOptions {
        plan: MissionPlanRequest::Survey(request),
        navdata: MissionNavdataSource::Fixture,
        anchor: GeoPointDegrees {
            lat_deg: LAUNCH.lat_deg,
            lon_deg: LAUNCH.lon_deg,
            alt_m: LAUNCH.alt_m,
        },
        date: None,
        cruise_height_m: Some(30.0),
        ownship_source: OwnshipSource::vehicle_estimate(),
        execution_target: ExecutionTarget::Simulator,
        cruise_mps: Some(8.0),
    }
}

#[tokio::test(start_paused = true)]
async fn a_vehicle_that_lands_during_the_climb_ends_the_run_with_the_reason() {
    let mut vehicle = SimConfig::new(LAUNCH);
    vehicle.name = "landing-test".to_owned();
    vehicle.camera.tiles = TileSourceConfig::Procedural;
    // A few seconds of hover: the pack empties well before 30 m.
    vehicle.battery.capacity_wh = 0.03;
    let mut runs = runtime::spawn_sim_multicopter_runs(vehicle);
    let mut observer = runs.observe().await;
    let status = runs.fly(&survey_options()).await.expect("the survey plans");
    let last = timeout(Duration::from_secs(300), async {
        loop {
            observer
                .next_datagram()
                .await
                .expect("the engine actor keeps broadcasting");
            let now = status.borrow().clone();
            if now.stopped {
                return now;
            }
            assert_ne!(now.mission_state, Some(MissionState::Complete), "{now:?}");
        }
    })
    .await
    .expect("the run ends");
    assert!(last.arm_accepted, "{last:?}");
    assert_eq!(
        last.ended_by_vehicle,
        Some("the vehicle landed outside the plan"),
        "{last:?}"
    );
    runs.shutdown().await;
}
