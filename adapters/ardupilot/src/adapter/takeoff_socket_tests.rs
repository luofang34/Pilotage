//! Real encoded observations and complete UDP writes drive the production adapter.

use std::time::Duration;

use pilotage_adapter_api::{
    Disposition, FlightProcedurePhase as Phase, LinkLossPolicy, VehicleAdapter,
};
use pilotage_mavlink::codec::{COMMAND_LONG_ID, HEARTBEAT_ID, MAV_CMD_NAV_TAKEOFF};
use pilotage_protocol::{
    AdmittedTiming, CommandOutcome, ControlAction, FlightProcedureReason, ScopeId, VehicleId,
};
use pilotage_timing::MonoTimestamp;

use super::FLIGHT_SCOPE;
use super::tests::{scoped, velocity};

mod device;
use device::{Harness, Observation, float, message_id, word};

fn run(test: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
        .block_on(test);
}

async fn request_takeoff(harness: &mut Harness, timeout: Duration) {
    let mut request = scoped(FLIGHT_SCOPE, None, vec![ControlAction::Takeoff]);
    request.admitted_timing = Some(AdmittedTiming {
        admitted_at: MonoTimestamp::from_nanos(0),
        send_remaining: Duration::from_millis(250),
        active_remaining: None,
        completion_timeout: Some(timeout),
        window_bounded: false,
    });
    let outcome = harness.adapter.apply_control(&request);
    assert_eq!(outcome.disposition, Disposition::Accepted);
    let packet = harness
        .receive(|frame| {
            message_id(frame) == COMMAND_LONG_ID && word(frame, 38) == MAV_CMD_NAV_TAKEOFF
        })
        .await;
    assert_eq!(&packet[40..42], &[23, 1]);
    assert_eq!(float(&packet, 34), 7.5);
    assert!(
        float(&packet, 22).is_nan(),
        "takeoff does not command heading"
    );
}

async fn await_phase(harness: &mut Harness, expected: Phase) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    tokio::time::timeout_at(deadline, async {
        loop {
            let phase = harness
                .adapter
                .sample_telemetry()
                .samples
                .iter()
                .find_map(|sample| sample.flight_procedure.as_ref().map(|status| status.phase));
            if phase == Some(expected) {
                return;
            }
            // Link presence is an event from the actual transport task.
            harness
                .receive(|frame| message_id(frame) == HEARTBEAT_ID)
                .await;
        }
    })
    .await
    .expect("procedure reaches expected state");
}

#[test]
fn accepted_ack_does_not_finish_takeoff_and_observed_climb_releases_velocity() {
    run(async {
        let mut harness = Harness::start().await;
        request_takeoff(&mut harness, Duration::from_secs(15)).await;
        harness
            .observation
            .send(Observation {
                ack: Some(0),
                ..Observation::default()
            })
            .unwrap();
        await_phase(&mut harness, Phase::InProgress).await;
        let request = scoped(
            FLIGHT_SCOPE,
            velocity(1.0, 0.0, 0.2).map(pilotage_protocol::ControlIntent::Velocity),
            vec![],
        );
        assert_eq!(
            harness.adapter.apply_control(&request).disposition,
            Disposition::Constrained
        );
        harness
            .observation
            .send(Observation {
                down_m: -2.5,
                altitude_mm: 7_500,
                landed: 2,
                ack: Some(0),
                ..Observation::default()
            })
            .unwrap();
        await_phase(&mut harness, Phase::Completed).await;
        assert_eq!(
            harness.adapter.apply_control(&request).disposition,
            Disposition::Accepted
        );
        let packet = harness.receive(|frame| message_id(frame) == 84).await;
        assert_eq!(float(&packet, 26), 1.0, "forward body velocity");
        harness.finish().await;
    });
}

#[test]
fn takeoff_timeout_is_reported_and_does_not_release_motion() {
    run(async {
        let mut harness = Harness::start().await;
        request_takeoff(&mut harness, Duration::from_secs(2)).await;
        harness
            .observation
            .send(Observation {
                ack: Some(0),
                ..Observation::default()
            })
            .unwrap();
        await_phase(&mut harness, Phase::TimedOut).await;
        let status = harness.adapter.sample_telemetry().samples[0]
            .flight_procedure
            .clone()
            .expect("terminal status");
        assert_eq!(status.reason, Some(FlightProcedureReason::DeadlineExpired));
        let request = scoped(
            FLIGHT_SCOPE,
            velocity(1.0, 0.0, 0.0).map(pilotage_protocol::ControlIntent::Velocity),
            vec![],
        );
        assert!(matches!(
            harness.adapter.apply_control(&request).disposition,
            Disposition::Rejected(_)
        ));
        harness.finish().await;
    });
}

#[test]
fn a_denied_takeoff_is_terminal_even_if_later_telemetry_reports_climb() {
    run(async {
        let mut harness = Harness::start().await;
        request_takeoff(&mut harness, Duration::from_secs(15)).await;
        harness
            .observation
            .send(Observation {
                ack: Some(2),
                ..Observation::default()
            })
            .unwrap();
        await_phase(&mut harness, Phase::Failed).await;
        let before = harness.adapter.sample_telemetry().samples[0]
            .flight_procedure
            .clone()
            .unwrap();
        assert_eq!(before.fc_outcome, Some(CommandOutcome::Denied));
        assert_eq!(before.reason, Some(FlightProcedureReason::CommandRefused));
        harness
            .observation
            .send(Observation {
                down_m: -2.5,
                altitude_mm: 7_500,
                landed: 2,
                ack: Some(0),
                ..Observation::default()
            })
            .unwrap();
        harness
            .receive(|frame| message_id(frame) == HEARTBEAT_ID)
            .await;
        let after = harness.adapter.sample_telemetry().samples[0]
            .flight_procedure
            .clone()
            .unwrap();
        assert_eq!(
            before, after,
            "a terminal result is not relabelled as success"
        );
        harness.finish().await;
    });
}

#[test]
fn revoking_a_written_takeoff_attempts_hold_without_claiming_the_vehicle_stopped() {
    run(async {
        let mut harness = Harness::start().await;
        request_takeoff(&mut harness, Duration::from_secs(15)).await;
        harness
            .adapter
            .invalidate_pending_commands(VehicleId::new(1), &ScopeId::new(FLIGHT_SCOPE));
        harness
            .adapter
            .set_link_loss_policy(
                VehicleId::new(1),
                &ScopeId::new(FLIGHT_SCOPE),
                Some(LinkLossPolicy::Neutralize),
            )
            .expect("zero demand queued");
        let packet = harness.receive(|frame| message_id(frame) == 84).await;
        assert_eq!(
            [float(&packet, 26), float(&packet, 30), float(&packet, 34)],
            [0.0; 3]
        );
        let status = harness.adapter.sample_telemetry().samples[0]
            .flight_procedure
            .clone()
            .unwrap();
        assert_eq!(status.phase, Phase::Cancelled);
        assert_eq!(status.reason, Some(FlightProcedureReason::AuthorityLost));
        harness.finish().await;
    });
}

#[test]
fn a_source_reset_after_takeoff_write_cannot_report_completion_or_release_motion() {
    run(async {
        let mut harness = Harness::start().await;
        request_takeoff(&mut harness, Duration::from_secs(15)).await;
        harness
            .observation
            .send(Observation {
                down_m: -2.5,
                altitude_mm: 7_500,
                landed: 2,
                ack: Some(0),
                restart: true,
            })
            .unwrap();
        // Observe the confirmed receive epoch before advancing the procedure;
        // intermediate reset quarantine can independently fail its measurement gate.
        harness.await_source_reset().await;
        let status = harness.adapter.sample_telemetry().samples[0]
            .flight_procedure
            .clone()
            .expect("procedure status after real source reset");
        assert_eq!(status.phase, Phase::Failed);
        assert_eq!(status.reason, Some(FlightProcedureReason::SourceChanged));
        let request = scoped(
            FLIGHT_SCOPE,
            velocity(1.0, 0.0, 0.0).map(pilotage_protocol::ControlIntent::Velocity),
            vec![],
        );
        assert!(matches!(
            harness.adapter.apply_control(&request).disposition,
            Disposition::Rejected(_)
        ));
        harness
            .receive(|frame| {
                let id = message_id(frame);
                assert_ne!(id, 84, "new-epoch altitude must not release velocity");
                assert_ne!(id, 82, "new-epoch altitude must not release direct control");
                id == HEARTBEAT_ID
            })
            .await;
        harness.finish().await;
    });
}
