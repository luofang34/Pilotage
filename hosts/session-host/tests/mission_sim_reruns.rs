//! Two survey runs in turn over one simulated multicopter, as a mission
//! lane flies a re-flight: the first run completes, stops and releases the
//! vehicle, and the second run takes the lease with the vehicle still in
//! the air, clears the link-loss latch the release left, and completes.

#![cfg(feature = "sim")]

use std::time::Duration;

use pilotage_adapter_sim_multicopter::{LaunchPoint, SimConfig, TileSourceConfig};
use pilotage_mission::fixture::GeoPointDegrees;
use pilotage_mission::{ExecutionTarget, MissionState, OwnshipSource, SurveyRequest};
use pilotage_session_host::runtime::{
    self, AutomationStatus, MissionNavdataSource, MissionOptions, MissionPlanRequest,
    TelemetryObserver,
};
use tokio::sync::watch;
use tokio::time::timeout;

const FLIGHT_TIMEOUT: Duration = Duration::from_secs(600);
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
    request.sidelap = 0.4;
    request.overshoot_m = 5.0;
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
        cruise_height_m: Some(15.0),
        ownship_source: OwnshipSource::vehicle_estimate(),
        execution_target: ExecutionTarget::Simulator,
        cruise_mps: Some(8.0),
    }
}

/// Drains broadcasts until the run ends; returns its final status.
#[cfg(test)]
async fn finish(
    observer: &mut TelemetryObserver,
    status: &watch::Receiver<AutomationStatus>,
) -> AutomationStatus {
    timeout(FLIGHT_TIMEOUT, async {
        loop {
            observer
                .next_datagram()
                .await
                .expect("the engine actor keeps broadcasting");
            let state = status.borrow().mission_state;
            if matches!(state, Some(MissionState::Complete | MissionState::Failed)) {
                return status.borrow().clone();
            }
        }
    })
    .await
    .expect("the run ends")
}

#[tokio::test(start_paused = true)]
async fn a_second_run_flies_after_the_first_released_the_airborne_vehicle() {
    let mut vehicle = SimConfig::new(LAUNCH);
    vehicle.name = "rerun-test".to_owned();
    vehicle.camera.tiles = TileSourceConfig::Procedural;
    let mut runs = runtime::spawn_sim_multicopter_runs(vehicle);
    let mut observer = runs.observe().await;
    for run in ["first", "second"] {
        let status = runs.fly(&survey_options()).await.expect("the survey plans");
        let last = finish(&mut observer, &status).await;
        assert_eq!(
            last.mission_state,
            Some(MissionState::Complete),
            "the {run} run: {last:?}"
        );
        assert!(last.arm_accepted, "the {run} run armed the vehicle");
        assert_eq!(last.frames_rejected, 0, "the {run} run: {last:?}");
    }
    runs.shutdown().await;
}
