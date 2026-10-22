#![allow(clippy::expect_used, clippy::panic)]

use super::*;

fn advertised(max_linear: f32) -> wire::HostCapabilities {
    wire::HostCapabilities {
        vehicles: vec![wire::VehicleDescriptor {
            vehicle: Some(wire::VehicleId {
                value: HOST_VEHICLE.as_u64(),
            }),
            scopes: vec![wire::ScopeDescriptor {
                payload: None,
                scope: Some(wire::ScopeId {
                    value: MISSION_SCOPE.to_owned(),
                }),
                intents: vec![wire::IntentCapability {
                    family: wire::IntentFamily::Velocity as i32,
                    max_linear,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn a_missing_velocity_capability_is_mission_fatal_not_fail_open() {
    let mut limits = MissionLimits::default();
    let configured = limits;
    assert!(!tighten_limits(
        &mut limits,
        &wire::HostCapabilities::default()
    ));
    assert_eq!(
        limits, configured,
        "refusal, not silent flight on unadvertised ceilings"
    );
}

#[test]
fn an_advertised_envelope_tightens_the_configured_ceilings() {
    let mut limits = MissionLimits::default();
    assert!(tighten_limits(&mut limits, &advertised(1.0)));
    assert_eq!(limits.max_horizontal_mps, 1.0);
}

/// A real vehicle's mission waits for its pilot's launch.
mod launch {
    use navigate_contract::GeodeticPosition;
    use pilotage_mission::{
        ExecutionTarget, MissionConfig, MissionEngine, OwnshipSource, SurveyRequest,
    };
    use pilotage_protocol::{Generation, LeaseResponse, PrincipalId, ScopeId, wire};
    use tokio::sync::{mpsc, watch};

    use super::super::super::{MISSION_SCOPE, MissionTask};
    use super::super::AIRBORNE_HEIGHT_M;
    use crate::runtime::{HOST_VEHICLE, automation::AutomationStatus};

    const LAT_DEG: f64 = 40.5442;
    const LON_DEG: f64 = -74.4565;

    /// A real-vehicle survey task after its welcome, waiting for the launch.
    fn waiting_task() -> (MissionTask, watch::Receiver<AutomationStatus>) {
        let (engine_tx, _engine_rx) = mpsc::channel(4);
        let (status_tx, status_rx) = watch::channel(AutomationStatus::default());
        let mut task = MissionTask::without_plan(
            engine_tx.downgrade(),
            tokio::time::Instant::now(),
            status_tx,
        );
        task.principal = Some(PrincipalId::new(9));
        task.ownship_source = OwnshipSource::vehicle_estimate();
        task.real_vehicle = true;
        let anchor = GeodeticPosition::new(LAT_DEG.to_radians(), LON_DEG.to_radians(), 20.0);
        let mut config = MissionConfig::new(
            String::new(),
            anchor,
            crate::runtime::automation::MISSION_CLOCK,
        );
        config.ownship_source = OwnshipSource::vehicle_estimate();
        config.execution_target = ExecutionTarget::RealVehicle;
        config.cruise_height_m = 110.0;
        let polygon = vec![
            [LAT_DEG, LON_DEG],
            [LAT_DEG + 0.002, LON_DEG],
            [LAT_DEG + 0.002, LON_DEG + 0.002],
            [LAT_DEG, LON_DEG + 0.002],
        ];
        let (engine, _) = MissionEngine::from_survey(&SurveyRequest::new(polygon), config)
            .expect("survey builds");
        task.mission = Some(engine);
        task.wait_for_launch();
        (task, status_rx)
    }

    /// The vehicle's own estimate at `height_m` above its launch point.
    fn estimate_at(height_m: f64, sequence: u32) -> wire::TelemetrySample {
        let stamp = wire::MeasurementStamp {
            role: wire::SourceRole::OperationalEstimate as i32,
            sequence,
            ..Default::default()
        };
        wire::TelemetrySample {
            vehicle: Some(wire::VehicleId {
                value: HOST_VEHICLE.as_u64(),
            }),
            avionics: Some(wire::AvionicsState {
                valid_flags: 0b1101,
                quat_w: (0.2_f32).cos(),
                quat_z: (0.2_f32).sin(),
                attitude_stamp: Some(stamp.clone()),
                quality: 1,
                kinematics_stamp: Some(stamp.clone()),
                estimator_status_stamp: Some(stamp.clone()),
                geodetic: Some(wire::GeodeticFix {
                    latitude_deg: LAT_DEG,
                    longitude_deg: LON_DEG,
                    horizontal_datum: 1,
                    height_m,
                    vertical_datum: 6,
                    local_origin: 77,
                    ..Default::default()
                }),
                geodetic_stamp: Some(stamp),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_real_vehicle_mission_asks_for_the_lease_only_once_airborne() {
        let (mut task, status) = waiting_task();
        assert!(task.held && task.awaiting_launch && !task.lease_due);
        assert!(status.borrow().awaiting_launch && status.borrow().held);
        task.on_telemetry(&estimate_at(AIRBORNE_HEIGHT_M - 1.0, 1));
        assert!(
            task.awaiting_launch && !task.lease_due,
            "on the ground: no lease"
        );
        task.on_telemetry(&estimate_at(AIRBORNE_HEIGHT_M + 1.0, 2));
        assert!(
            !task.awaiting_launch && task.lease_due,
            "airborne: the lease is due"
        );
        assert!(!status.borrow().awaiting_launch);
        assert!(task.held, "held until the lease is granted");
        task.on_lease_response(&LeaseResponse {
            vehicle: HOST_VEHICLE,
            scope: ScopeId::new(MISSION_SCOPE),
            granted: true,
            generation: Generation::new(5),
            reason: None,
        });
        assert!(!task.held);
        assert_eq!(task.generation, Some(Generation::new(5)));
        assert_eq!(
            task.neutral_frames, 1,
            "the first frame under the lease is neutral"
        );
        assert_eq!(status.borrow().resumes, 1);
    }

    #[tokio::test]
    async fn a_denied_lease_after_the_launch_holds_the_mission_again() {
        let (mut task, _status) = waiting_task();
        task.on_telemetry(&estimate_at(AIRBORNE_HEIGHT_M + 5.0, 1));
        task.on_lease_response(&LeaseResponse {
            vehicle: HOST_VEHICLE,
            scope: ScopeId::new(MISSION_SCOPE),
            granted: false,
            generation: Generation::new(0),
            reason: None,
        });
        assert!(task.held && task.generation.is_none());
    }
}
