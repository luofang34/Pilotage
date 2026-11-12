//! A waypoint-list mission over the simulated multicopter, wired as the
//! host runtime wires `PILOTAGE_MISSION_WAYPOINTS` with
//! `--adapter sim-multicopter`: the mission arms the vehicle, climbs,
//! flies to each waypoint at its own altitude, and completes. The test
//! runs under a paused clock, and every wait is event-driven on the
//! broadcast telemetry under a bounded virtual-time timeout.

#![cfg(feature = "sim")]

use std::time::Duration;

use pilotage_adapter_sim_multicopter::{LaunchPoint, SimConfig, TileSourceConfig};
use pilotage_mission::fixture::GeoPointDegrees;
use pilotage_mission::{
    ExecutionTarget, MissionState, OwnshipSource, PlannedWaypoint, WaypointPlan,
};
use pilotage_protocol::wire;
use pilotage_session_host::runtime::{
    self, MissionNavdataSource, MissionOptions, MissionPlanRequest,
};
use prost::Message;
use tokio::time::timeout;

const FLIGHT_TIMEOUT: Duration = Duration::from_secs(600);
const LAUNCH: LaunchPoint = LaunchPoint {
    lat_deg: 40.3317,
    lon_deg: -74.6745,
    alt_m: 40.0,
};

/// Two waypoints north and north-east of the launch point, at 15 m and
/// 22 m above it.
fn waypoints() -> WaypointPlan {
    let north = 60.0 / 111_320.0;
    let east = 40.0 / (111_320.0 * LAUNCH.lat_deg.to_radians().cos());
    let mut plan = WaypointPlan::new(vec![
        PlannedWaypoint::new(LAUNCH.lat_deg + north, LAUNCH.lon_deg, LAUNCH.alt_m + 15.0),
        PlannedWaypoint::new(
            LAUNCH.lat_deg + north,
            LAUNCH.lon_deg + east,
            LAUNCH.alt_m + 22.0,
        ),
    ]);
    plan.capture_radius_m = Some(3.0);
    plan
}

fn options() -> MissionOptions {
    MissionOptions {
        plan: MissionPlanRequest::Waypoints(waypoints()),
        navdata: MissionNavdataSource::Fixture,
        anchor: GeoPointDegrees {
            lat_deg: LAUNCH.lat_deg,
            lon_deg: LAUNCH.lon_deg,
            alt_m: LAUNCH.alt_m,
        },
        date: None,
        cruise_height_m: Some(10.0),
        ownship_source: OwnshipSource::vehicle_estimate(),
        execution_target: ExecutionTarget::Simulator,
        cruise_mps: Some(6.0),
    }
}

/// The vehicle's last estimated position.
fn fix_of(bytes: &[u8]) -> Option<wire::GeodeticFix> {
    let envelope = wire::Envelope::decode(bytes).ok()?;
    match envelope.payload? {
        wire::envelope::Payload::TelemetrySample(sample) => sample.avionics?.geodetic,
        _ => None,
    }
}

#[tokio::test(start_paused = true)]
async fn a_waypoint_list_flies_to_each_waypoint_and_completes() {
    let mut vehicle = SimConfig::new(LAUNCH);
    vehicle.name = "waypoint-test".to_owned();
    vehicle.camera.tiles = TileSourceConfig::Procedural;
    let rig = runtime::spawn_sim_multicopter_rig(vehicle, options()).expect("the rig spawns");
    let mut observer = rig.observe().await;
    let status = rig.status();
    let mut last_fix = None;
    let mut seen = Vec::new();
    timeout(FLIGHT_TIMEOUT, async {
        loop {
            let datagram = observer
                .next_datagram()
                .await
                .expect("the engine actor keeps broadcasting");
            if let Some(fix) = fix_of(&datagram) {
                last_fix = Some(fix);
            }
            let state = status.borrow().mission_state;
            if let Some(state) = state
                && seen.last() != Some(&state)
            {
                seen.push(state);
            }
            if state == Some(MissionState::Complete) || state == Some(MissionState::Failed) {
                break;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the waypoint list did not finish: {seen:?}"));
    let final_status = status.borrow().clone();
    assert_eq!(
        final_status.mission_state,
        Some(MissionState::Complete),
        "{seen:?}"
    );
    assert!(final_status.arm_accepted, "the vehicle confirmed the arm");
    assert!(final_status.plan_complete);
    let fix = last_fix.expect("the vehicle reported its position");
    let last = &waypoints().waypoints[1];
    let north_m = (fix.latitude_deg - last.latitude_deg).to_radians() * 6_371_000.0;
    let east_m = (fix.longitude_deg - last.longitude_deg).to_radians()
        * 6_371_000.0
        * last.latitude_deg.to_radians().cos();
    assert!(
        north_m.hypot(east_m) < 3.0,
        "the vehicle ends at the last waypoint: {north_m} m north, {east_m} m east"
    );
    // The simulated vehicle reports its height on the ellipsoid or above
    // its launch point; the waypoint altitude is on the ellipsoid.
    let last_alt_m = last.altitude.reference_m();
    let expected_m = match fix.vertical_datum {
        1 => last_alt_m,
        6 => last_alt_m - LAUNCH.alt_m,
        other => panic!("an unexpected vertical datum {other}"),
    };
    assert!(
        (fix.height_m - expected_m).abs() < 3.0,
        "the vehicle holds the last waypoint's altitude: {} m, planned {expected_m} m",
        fix.height_m
    );
}
