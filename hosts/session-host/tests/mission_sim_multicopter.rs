//! In-process area survey over the simulated multicopter: the engine
//! actor and the mission principal wired as the host runtime wires them
//! for `--adapter sim-multicopter`, with the mission reading the vehicle's
//! own estimate as it does for the DJI aircraft.
//!
//! The mission arms the vehicle, climbs with velocity intents, flies every
//! survey leg, and completes over the launch point. The test runs under a
//! paused clock, and every wait is event-driven on the broadcast telemetry
//! under a bounded virtual-time timeout.

#![cfg(feature = "sim")]

use std::time::Duration;

use pilotage_adapter_sim_multicopter::{LaunchPoint, SimConfig, TileSourceConfig};
use pilotage_mission::fixture::GeoPointDegrees;
use pilotage_mission::{ExecutionTarget, MissionState, OwnshipSource, SurveyRequest};
use pilotage_protocol::wire;
use pilotage_session_host::runtime::{
    self, MissionNavdataSource, MissionOptions, MissionPlanRequest,
};
use prost::Message;
use tokio::time::timeout;

/// The survey flight bound in virtual time.
const FLIGHT_TIMEOUT: Duration = Duration::from_secs(600);
/// The survey height above the launch point, metres.
const SURVEY_HEIGHT_M: f64 = 15.0;
const LAUNCH: LaunchPoint = LaunchPoint {
    lat_deg: 40.3317,
    lon_deg: -74.6745,
    alt_m: 40.0,
};

/// A 50 m by 60 m rectangle north-east of the launch point, flown as two
/// north-south lines 24 m apart.
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
        anchor: navigate_anchor(),
        date: None,
        cruise_height_m: Some(SURVEY_HEIGHT_M),
        ownship_source: OwnshipSource::vehicle_estimate(),
        execution_target: ExecutionTarget::Simulator,
        cruise_mps: Some(8.0),
    }
}

fn navigate_anchor() -> GeoPointDegrees {
    GeoPointDegrees {
        lat_deg: LAUNCH.lat_deg,
        lon_deg: LAUNCH.lon_deg,
        alt_m: LAUNCH.alt_m,
    }
}

/// What the broadcast telemetry showed during the flight.
#[derive(Debug, Default)]
struct Flight {
    max_height_m: f64,
    last_fix: Option<wire::GeodeticFix>,
    max_leg: u32,
    waypoint_count: u32,
    estimates: u64,
}

impl Flight {
    fn observe(&mut self, bytes: &[u8]) {
        let Ok(envelope) = wire::Envelope::decode(bytes) else {
            return;
        };
        let Some(wire::envelope::Payload::TelemetrySample(sample)) = envelope.payload else {
            return;
        };
        if let Some(guidance) = sample.nav_guidance.as_deref() {
            self.max_leg = self.max_leg.max(guidance.leg_index);
            self.waypoint_count = guidance.waypoint_count;
        }
        if let Some(fix) = sample.avionics.as_ref().and_then(|a| a.geodetic.clone()) {
            self.max_height_m = self.max_height_m.max(fix.height_m);
            self.last_fix = Some(fix);
            self.estimates += 1;
        }
    }
}

/// Horizontal metres between two WGS-84 points a few hundred metres apart.
fn distance_m(lat0: f64, lon0: f64, lat1: f64, lon1: f64) -> f64 {
    let north = (lat1 - lat0).to_radians() * 6_371_000.0;
    let east = (lon1 - lon0).to_radians() * 6_371_000.0 * lat0.to_radians().cos();
    north.hypot(east)
}

#[tokio::test(start_paused = true)]
async fn the_survey_arms_climbs_flies_every_leg_and_ends_over_home() {
    let mut vehicle = SimConfig::new(LAUNCH);
    vehicle.name = "survey-test".to_owned();
    vehicle.camera.tiles = TileSourceConfig::Procedural;
    let rig =
        runtime::spawn_sim_multicopter_rig(vehicle, survey_options()).expect("the rig spawns");
    let mut observer = rig.observe().await;
    let status = rig.status();
    let mut flight = Flight::default();
    let mut seen = Vec::new();
    timeout(FLIGHT_TIMEOUT, async {
        loop {
            let datagram = observer
                .next_datagram()
                .await
                .expect("the engine actor keeps broadcasting");
            flight.observe(&datagram);
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
    .unwrap_or_else(|_| panic!("the survey did not finish: {seen:?} {flight:?}"));
    let final_status = status.borrow().clone();
    assert_eq!(
        final_status.mission_state,
        Some(MissionState::Complete),
        "{seen:?}"
    );
    assert!(final_status.arm_accepted, "the mission armed the vehicle");
    assert_eq!(final_status.frames_rejected, 0);
    for phase in [MissionState::Climb, MissionState::Enroute] {
        assert!(seen.contains(&phase), "{phase:?} not seen in {seen:?}");
    }
    assert!(
        (flight.max_height_m - SURVEY_HEIGHT_M).abs() < 2.0,
        "climbed to {} m",
        flight.max_height_m
    );
    assert!(flight.waypoint_count >= 5, "{flight:?}");
    assert_eq!(
        flight.max_leg + 1,
        flight.waypoint_count,
        "every survey waypoint became the active leg"
    );
    let fix = flight.last_fix.expect("the vehicle published its estimate");
    let home = distance_m(
        LAUNCH.lat_deg,
        LAUNCH.lon_deg,
        fix.latitude_deg,
        fix.longitude_deg,
    );
    assert!(home < 8.0, "the survey ended {home} m from home");
    assert!(
        (fix.height_m - SURVEY_HEIGHT_M).abs() < 2.0,
        "{}",
        fix.height_m
    );
    rig.shutdown().await;
}
