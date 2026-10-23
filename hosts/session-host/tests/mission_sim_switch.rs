//! A successor run takes over mid-air, as a mission lane switches to an
//! admitted revision: the first run flies its first line, and at that
//! checkpoint the lane stops it and starts the successor over the area.
//! The vehicle must keep its survey height through the switch, and the
//! successor must arm, fly and complete without a rejected frame.

#![cfg(feature = "sim")]

use std::time::Duration;

use pilotage_adapter_sim_multicopter::{LaunchPoint, SimConfig, TileSourceConfig};
use pilotage_mission::fixture::GeoPointDegrees;
use pilotage_mission::{ExecutionTarget, MissionState, OwnshipSource, SurveyRequest};
use pilotage_protocol::wire;
use pilotage_session_host::runtime::{
    self, AutomationStatus, MissionNavdataSource, MissionOptions, MissionPlanRequest,
    TelemetryObserver,
};
use prost::Message as _;
use tokio::sync::watch;
use tokio::time::timeout;

const FLIGHT_TIMEOUT: Duration = Duration::from_secs(900);
const HEIGHT_M: f64 = 15.0;
const LAUNCH: LaunchPoint = LaunchPoint {
    lat_deg: 40.3317,
    lon_deg: -74.6745,
    alt_m: 40.0,
};

/// A survey of the box `east_m` wide starting `east0_m` east of launch.
#[cfg(test)]
fn survey(east0_m: f64, east_m: f64) -> MissionOptions {
    let north = 60.0 / 111_320.0;
    let metre_east = 1.0 / (111_320.0 * LAUNCH.lat_deg.to_radians().cos());
    let (lat, lon) = (LAUNCH.lat_deg, LAUNCH.lon_deg + east0_m * metre_east);
    let width = east_m * metre_east;
    let mut request = SurveyRequest::new(vec![
        [lat, lon],
        [lat + north, lon],
        [lat + north, lon + width],
        [lat, lon + width],
    ]);
    request.footprint_width_m = Some(20.0);
    request.sidelap = 0.2;
    request.line_bearing_rad = Some(0.0);
    request.fit_to_cruise(8.0);
    MissionOptions {
        plan: MissionPlanRequest::Survey(request),
        navdata: MissionNavdataSource::Fixture,
        anchor: GeoPointDegrees {
            lat_deg: LAUNCH.lat_deg,
            lon_deg: LAUNCH.lon_deg,
            alt_m: LAUNCH.alt_m,
        },
        date: None,
        cruise_height_m: Some(HEIGHT_M),
        ownship_source: OwnshipSource::vehicle_estimate(),
        execution_target: ExecutionTarget::Simulator,
        cruise_mps: Some(8.0),
    }
}

/// The height above launch a broadcast reports, when it carries one.
#[cfg(test)]
fn height_of(bytes: &[u8]) -> Option<f64> {
    let envelope = wire::Envelope::decode(bytes).ok()?;
    let Some(wire::envelope::Payload::TelemetrySample(sample)) = envelope.payload else {
        return None;
    };
    sample.avionics.map(|a| -f64::from(a.pos_d_m))
}

/// Drains broadcasts until `done` holds for the run's status; returns the
/// lowest height seen meanwhile.
#[cfg(test)]
async fn until(
    observer: &mut TelemetryObserver,
    status: &watch::Receiver<AutomationStatus>,
    done: impl Fn(&AutomationStatus) -> bool,
) -> f64 {
    timeout(FLIGHT_TIMEOUT, async {
        let mut lowest = f64::INFINITY;
        loop {
            let bytes = observer.next_datagram().await.expect("broadcasts continue");
            if let Some(height) = height_of(&bytes) {
                lowest = lowest.min(height);
            }
            if done(&status.borrow()) {
                return lowest;
            }
        }
    })
    .await
    .expect("the awaited state arrives")
}

#[tokio::test(start_paused = true)]
async fn a_successor_run_takes_over_mid_air_at_a_line_end() {
    let mut vehicle = SimConfig::new(LAUNCH);
    vehicle.name = "switch-test".to_owned();
    vehicle.camera.tiles = TileSourceConfig::Procedural;
    let mut runs = runtime::spawn_sim_multicopter_runs(vehicle);
    let mut observer = runs.observe().await;

    let first = runs
        .fly(&survey(20.0, 64.0))
        .await
        .expect("the first survey plans");
    // Line 1 is flown toward waypoint 1; waypoint 2 is the start of line 2.
    until(&mut observer, &first, |s| s.target_waypoint == Some(2)).await;
    assert_eq!(first.borrow().mission_state, Some(MissionState::Enroute));

    // The checkpoint: the successor flies the far half of the area.
    let second = runs
        .fly(&survey(52.0, 32.0))
        .await
        .expect("the successor plans");
    let lowest = until(&mut observer, &second, |s| {
        matches!(
            s.mission_state,
            Some(MissionState::Complete | MissionState::Failed)
        )
    })
    .await;
    let last = second.borrow().clone();
    assert_eq!(last.mission_state, Some(MissionState::Complete), "{last:?}");
    assert!(last.arm_accepted, "the successor armed: {last:?}");
    assert_eq!(last.frames_rejected, 0, "{last:?}");
    assert!(
        lowest > HEIGHT_M - 3.0,
        "the vehicle sank to {lowest:.1} m between runs"
    );
    runs.shutdown().await;
}
