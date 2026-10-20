use super::*;
use crate::gimbal_endpoint::{GimbalStamp, SCOPE, apply_control, sample, scope_descriptor};
use pilotage_adapter_api::{Disposition, SourceIncarnation};
use pilotage_protocol::{
    ControlIntent, ControlPayload, Generation, GimbalRateIntent, ScopeId, ScopedControlFrame,
    SequenceNum, SessionId, VehicleId, wire,
};
use pilotage_timing::{MonoTimestamp, SimTick};

fn frame() -> ScopedControlFrame {
    ScopedControlFrame {
        timing: None,
        admitted_timing: None,
        session: SessionId::new(1),
        vehicle: VehicleId::new(1),
        scope: ScopeId::new(SCOPE),
        generation: Generation::new(1),
        sequence: SequenceNum::new(1),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 0,
        activation_revision: 0,
        payload: ControlPayload::default(),
        intent: Some(ControlIntent::GimbalRate(GimbalRateIntent {
            pitch_rate: 0.4,
            yaw_rate: 0.0,
        })),
        actions: vec![],
        action_ids: vec![],
    }
}

async fn send_projected_rate(fixture: &mut Fixture) {
    let outcome = apply_control(
        &mut fixture.control,
        VehicleId::new(1),
        &frame(),
        SimTick::new(0),
    );
    assert_eq!(outcome.disposition, Disposition::Accepted);
    let demand = fixture.rates.borrow().clone().unwrap();
    fixture.send_rate(demand).await;
    assert_eq!(receive_message(&fixture.receiver).await, 282);
}

fn assert_projection(
    fixture: &Fixture,
    stamp: &GimbalStamp,
    epoch: u64,
    sent: bool,
) -> wire::PayloadState {
    let advertisement = scope_descriptor(&fixture.control, stamp).payload.unwrap();
    let payload = sample(&fixture.control, stamp).payload.unwrap();
    assert_eq!(advertisement.connection_epoch, epoch);
    assert_eq!(payload.connection_epoch, epoch);
    assert_eq!(
        payload.connection_incarnation,
        advertisement.connection_incarnation
    );
    assert_eq!(payload.capability_revision, advertisement.revision);
    for axis in [&payload.pitch, &payload.roll, &payload.yaw] {
        assert_eq!(axis.as_ref().unwrap().connection_epoch, epoch);
    }
    if sent {
        let command = payload.command.as_ref().unwrap();
        assert_eq!(command.connection_epoch, epoch);
        assert!(command.sent_at.is_some());
        assert_eq!(command.stage, wire::PayloadCommandStage::Sent as i32);
        assert!(command.observed_complete_at.is_none());
    } else {
        assert!(payload.command.is_none());
    }
    payload
}

async fn reset_projection(restart: bool) {
    let mut fixture = Fixture::new(restart, OWNER).await;
    let stamp = GimbalStamp::new(SourceIncarnation::new([7; 16]));
    feedback(&fixture.state, (1, KEY.1), 0, (0, 0), 102, Instant::now());
    send_projected_rate(&mut fixture).await;
    let before = assert_projection(&fixture, &stamp, 1, true);
    let old = fixture.control.snapshot_cached().unwrap();
    finish_source_reset(&fixture.state, restart);
    discovered(&fixture.state, 103, OWNER);
    feedback(&fixture.state, (1, KEY.1), 0, (0, 0), 105, Instant::now());
    let before_binding = sample(&fixture.control, &stamp).payload.unwrap();
    assert_eq!(before_binding.connection_epoch, before.connection_epoch);
    assert!(before_binding.pitch.is_none());
    assert!(before_binding.roll.is_none());
    assert!(before_binding.yaw.is_none());
    fixture.control.maintain();
    let epoch = fixture.control.connection_epoch();
    assert_ne!(epoch, before.connection_epoch);
    let retired = fixture.control.snapshot_cached().unwrap();
    assert_eq!(retired.command.identity, old.command.identity);
    assert_eq!(retired.progress.sent_at, old.progress.sent_at);
    assert_projection(&fixture, &stamp, epoch, false);
    send_projected_rate(&mut fixture).await;
    assert_projection(&fixture, &stamp, epoch, true);
}

async fn repeated_empty_resets() {
    let mut fixture = Fixture::new(false, OWNER).await;
    let sent = fixture.sent_rate().await;
    let stamp = GimbalStamp::new(SourceIncarnation::new([7; 16]));
    fixture.reset_and_rediscover(false, OWNER);
    let mut epoch = fixture.control.connection_epoch();
    for time in [110, 120] {
        for boot in [1 << 30, 1 << 31, 3 << 30, u32::MAX - 1] {
            attitude(&fixture.state, boot, Instant::now());
        }
        finish_source_reset(&fixture.state, false);
        discovered(&fixture.state, time, OWNER);
        fixture.control.maintain();
        assert_eq!(fixture.control.connection_epoch(), epoch.wrapping_add(1));
        epoch = fixture.control.connection_epoch();
        let retired = fixture.control.snapshot_cached().unwrap();
        assert_eq!(retired.command.identity, sent.command.identity);
        assert_eq!(retired.progress.sent_at, sent.progress.sent_at);
        assert_eq!(
            retired.progress.ended,
            Some(turret_core::EndReason::Revoked)
        );
        assert!(retired.progress.observed_complete_at.is_none());
        assert!(sample(&fixture.control, &stamp).payload.is_none());
        fixture.assert_no_control_write().await;
    }
    feedback(&fixture.state, (1, KEY.1), 0, (0, 0), 130, Instant::now());
    assert_projection(&fixture, &stamp, epoch, false);
}

#[test]
fn advertised_observed_axis_and_sent_command_epochs_match_after_raw_source_reset() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        for restart in [false, true] {
            reset_projection(restart).await;
        }
    });
}

#[test]
fn empty_connections_preserve_the_last_terminal_evidence_without_projecting_it_as_new() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(repeated_empty_resets());
}
