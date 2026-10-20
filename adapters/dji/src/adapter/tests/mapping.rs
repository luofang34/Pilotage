use pilotage_adapter_api::{Disposition, RejectReason, VehicleAdapter};
use pilotage_client_session::payload_gimbal_rate_intent;
use pilotage_protocol::{ControlIntent, GimbalRateIntent, ScopedControlFrame, wire};

use super::TestResult;
use super::harness::{Harness, frame, observed_poses, packets};

pub(super) fn mapped_frame(harness: &Harness, pitch: f32, yaw: f32) -> TestResultFrame {
    let capabilities = harness.adapter.capabilities();
    let advertised = &capabilities.vehicles[0].scopes[0];
    let scope = pilotage_client_session::ScopeCatalog {
        payload: advertised.payload.clone(),
        scope: advertised.scope.as_str().to_owned(),
        intents: vec![wire::IntentCapability {
            family: wire::IntentFamily::GimbalRate as i32,
            max_angular: advertised.intents[0].max_angular,
            ..Default::default()
        }],
        actions: Vec::new(),
        authority_group: advertised.scope.as_str().to_owned(),
    };
    let intent = payload_gimbal_rate_intent(pitch, yaw, Some(&scope))
        .ok_or("advertised gimbal intent was not mapped")?;
    let Some(wire::control_intent::Family::GimbalRate(rate)) = intent.family else {
        return Err("gimbal mapping produced a different intent family".into());
    };
    let mut request = frame(rate.pitch_rate);
    request.intent = Some(ControlIntent::GimbalRate(GimbalRateIntent {
        pitch_rate: rate.pitch_rate,
        yaw_rate: rate.yaw_rate,
    }));
    Ok(request)
}

type TestResultFrame = Result<ScopedControlFrame, Box<dyn std::error::Error>>;

#[test]
fn a_diagonal_shared_mapping_commands_bounded_pitch_without_yaw() -> TestResult {
    let mut harness = Harness::new();
    harness.prime()?;
    let request = mapped_frame(&harness, -1.0, 1.0)?;
    assert_eq!(
        harness.apply(request, 20).disposition,
        Disposition::Transformed
    );
    harness.step(30);
    let written = harness.written()?;
    let packets = packets(&written);
    assert_eq!(packets.len(), 2);
    assert_eq!(packets[1].payload(), &[0, 0, 0, 0, 206, 255, 7, 10]);
    Ok(())
}

#[test]
fn unsupported_yaw_maps_to_neutral_without_starting_motion_and_stops_active_pitch() -> TestResult {
    let mut harness = Harness::new();
    harness.prime()?;
    let before = harness.written()?;
    let request = mapped_frame(&harness, 0.0, 1.0)?;
    assert_eq!(
        harness.apply(request, 20).disposition,
        Disposition::Accepted
    );
    harness.step(30);
    assert_eq!(harness.written()?, before);
    let request = mapped_frame(&harness, -1.0, 1.0)?;
    harness.apply(request, 30);
    harness.step(40);
    let request = mapped_frame(&harness, 0.0, -1.0)?;
    assert_eq!(
        harness.apply(request, 40).disposition,
        Disposition::Transformed
    );
    harness.step(50);
    let written = harness.written()?;
    let packets = packets(&written);
    assert_eq!(packets.len(), 3);
    assert_eq!(packets[2].payload(), &[0, 0, 0, 0, 207, 255, 7, 10]);
    Ok(())
}

#[test]
fn nonfinite_yaw_is_rejected_without_a_write() -> TestResult {
    let mut harness = Harness::new();
    harness.prime()?;
    let before = harness.written()?;
    for yaw in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut request = frame(0.0);
        request.intent = Some(ControlIntent::GimbalRate(GimbalRateIntent {
            pitch_rate: 0.0,
            yaw_rate: yaw,
        }));
        assert!(matches!(
            harness.apply(request, 20).disposition,
            Disposition::Rejected(_)
        ));
        harness.step(30);
    }
    assert_eq!(harness.written()?, before);
    Ok(())
}

#[test]
fn interrupted_reads_cannot_turn_a_queued_backlog_into_fresh_arrivals() -> TestResult {
    let mut harness = Harness::new();
    let poses = observed_poses();
    harness.feed(&poses.concat())?;
    harness.set_read_limit(poses[0].len())?;
    for millis in [0, 10] {
        harness.interrupt_after_read()?;
        harness.step(millis);
    }
    harness.step(20);
    assert_eq!(harness.adapter.status_cached().consecutive_poses, 1);
    assert_eq!(
        harness.apply(frame(0.0), 20).disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    );
    Ok(())
}
