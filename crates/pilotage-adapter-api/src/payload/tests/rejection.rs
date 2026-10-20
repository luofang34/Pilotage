use super::super::payload_progress_to_wire;
use pilotage_protocol::{
    SCHEMA_VERSION, decode_envelope_length_delimited, encode_envelope_length_delimited, wire,
};
use turret_core::{
    Angle, AngleRange, Axes, AxisCapabilities, Capability, Command, CommandIdentity,
    CommandProgress, CommandTiming, Effect, Executor, Frames, GimbalCapabilities, GimbalPosition,
    Implementation, PayloadCapabilities, PayloadCommand, PositionCapability, ReferenceFrame,
    StopGuarantee, Tick,
};

#[test]
fn rejection_survives_expiry_and_revocation_on_the_wire() {
    let (mut executor, command) = sent();
    let rejected = executor
        .acknowledge(command.identity, false, Tick(150))
        .unwrap();
    assert_eq!(rejected.effect, Effect::CancelAction);
    let wire = roundtrip(rejected.progress, command, Tick(150));
    assert_rejection(&wire, wire::PayloadEndReason::DeviceRejected);
    assert!(!wire.acknowledgement_timed_out);

    let expired = executor.poll(Tick(300)).unwrap().unwrap();
    assert_eq!(expired.effect, Effect::CancelAction);
    let wire = roundtrip(expired.progress, command, Tick(300));
    assert_rejection(&wire, wire::PayloadEndReason::ValidityExpired);
    assert!(!wire.acknowledgement_timed_out);

    let revoked = executor.revoke().active.unwrap();
    assert_eq!(revoked.effect, Effect::CancelAction);
    let wire = roundtrip(revoked.progress, command, Tick(300));
    assert_rejection(&wire, wire::PayloadEndReason::Revoked);
    assert!(!wire.acknowledgement_timed_out);
}

#[test]
fn timely_rejection_does_not_become_an_acknowledgement_timeout() {
    let (mut executor, command) = sent();
    let rejected = executor
        .acknowledge(command.identity, false, Tick(150))
        .unwrap();
    assert_eq!(rejected.effect, Effect::CancelAction);
    assert_eq!(executor.poll(Tick(200)).unwrap(), None);
    let wire = roundtrip(
        executor.progress(command.identity).unwrap(),
        command,
        Tick(200),
    );
    assert_rejection(&wire, wire::PayloadEndReason::DeviceRejected);
    assert!(!wire.acknowledgement_timed_out);
}

#[test]
fn delayed_rejection_retains_the_missed_acknowledgement_deadline() {
    let (mut executor, command) = sent();
    let timeout = executor.poll(Tick(200)).unwrap().unwrap();
    assert!(timeout.progress.acknowledgement_timed_out);
    let rejected = executor
        .acknowledge(command.identity, false, Tick(250))
        .unwrap();
    let wire = roundtrip(rejected.progress, command, Tick(250));
    assert_eq!(wire.rejected_at.map(|value| value.nanos), Some(250_000));
    assert_eq!(wire.stage, wire::PayloadCommandStage::Rejected as i32);
    assert!(wire.acknowledgement_timed_out);
    assert_eq!(wire.acknowledged_at, None);
}

#[test]
fn rejection_does_not_erase_an_earlier_positive_acknowledgement() {
    let (mut executor, command) = sent();
    let accepted = executor
        .acknowledge(command.identity, true, Tick(40))
        .unwrap();
    let wire = roundtrip(accepted.progress, command, Tick(40));
    assert_eq!(wire.stage, wire::PayloadCommandStage::DeviceAccepted as i32);
    assert_eq!(wire.rejected_at, None);
    let rejected = executor
        .acknowledge(command.identity, false, Tick(150))
        .unwrap();
    let wire = roundtrip(rejected.progress, command, Tick(150));
    assert_rejection(&wire, wire::PayloadEndReason::DeviceRejected);
    assert_eq!(wire.acknowledged_at.map(|value| value.nanos), Some(40_000));
    assert_eq!(wire.observed_complete_at, None);
}

#[cfg(test)]
fn assert_rejection(value: &wire::PayloadCommandProgress, reason: wire::PayloadEndReason) {
    assert_eq!(value.stage, wire::PayloadCommandStage::Rejected as i32);
    assert_eq!(value.end_reason, reason as i32);
    assert_eq!(value.rejected_at.map(|value| value.nanos), Some(150_000));
    assert_eq!(value.connection_epoch, 9);
    assert_eq!(value.generation, 4);
    assert_eq!(value.command_id, 3);
    assert_eq!(value.sequence, 5);
    assert_eq!(value.admitted_at.map(|value| value.nanos), Some(10_000));
    assert_eq!(value.prepared_at.map(|value| value.nanos), Some(20_000));
    assert_eq!(value.sent_at.map(|value| value.nanos), Some(30_000));
    assert_eq!(value.frame, wire::PayloadReferenceFrame::Mount as i32);
    assert_eq!(value.pitch_target_rad, Some(0.0));
    assert_eq!(value.roll_target_rad, None);
    assert_eq!(value.yaw_target_rad, None);
    assert_eq!(value.observed_complete_at, None);
}

#[cfg(test)]
fn roundtrip(
    progress: CommandProgress,
    command: Command,
    now: Tick,
) -> wire::PayloadCommandProgress {
    let state = wire::PayloadState {
        connection_incarnation: vec![7; 16],
        connection_epoch: progress.identity.epoch,
        endpoint_id: "gimbal".to_owned(),
        capability_revision: command.capability_revision,
        command: Some(payload_progress_to_wire(&progress, Some(&command), now)),
        ..Default::default()
    };
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::TelemetrySample(
            wire::TelemetrySample {
                payload: Some(Box::new(state.clone())),
                ..Default::default()
            },
        )),
    };
    let bytes = encode_envelope_length_delimited(&envelope);
    let (decoded, remainder) = decode_envelope_length_delimited(&bytes).unwrap();
    assert!(remainder.is_empty());
    assert_eq!(decoded, envelope);
    let Some(wire::envelope::Payload::TelemetrySample(sample)) = decoded.payload else {
        panic!("expected telemetry");
    };
    let decoded_state = sample.payload.unwrap();
    assert_eq!(*decoded_state, state);
    decoded_state.command.unwrap()
}

#[cfg(test)]
fn sent() -> (Executor, Command) {
    let command = Command {
        identity: CommandIdentity {
            epoch: 9,
            generation: 4,
            id: 3,
            sequence: 5,
        },
        capability_revision: 4,
        payload: PayloadCommand::GimbalPosition(GimbalPosition {
            angles: Axes {
                pitch: Some(Angle::ZERO),
                roll: None,
                yaw: None,
            },
            frame: ReferenceFrame::Mount,
        }),
        timing: CommandTiming {
            send_before: Tick(100),
            valid_until: Some(Tick(300)),
            ack_before: Some(Tick(200)),
            completion_before: None,
        },
    };
    let mut executor = Executor::new(capabilities(), 9, 4);
    assert_eq!(executor.submit(command, Tick(10)).unwrap(), None);
    assert_eq!(executor.prepare_send(Tick(20)).unwrap(), Some(command));
    let sent = executor.record_sent(command.identity, Tick(30)).unwrap();
    assert_eq!(sent.current.effect, Effect::None);
    (executor, command)
}

#[cfg(test)]
fn capabilities() -> PayloadCapabilities {
    let pitch = AxisCapabilities {
        position: Capability::Available(PositionCapability {
            implementation: Implementation::Native,
            range: AngleRange {
                minimum: Angle::radians(-1.5).unwrap(),
                maximum: Angle::radians(0.5).unwrap(),
            },
            frames: Frames {
                mount: true,
                vehicle: false,
                earth_ned: false,
                horizon_vehicle_yaw: false,
            },
            maximum_rate: None,
        }),
        ..AxisCapabilities::UNSUPPORTED
    };
    PayloadCapabilities {
        revision: 4,
        gimbal: GimbalCapabilities {
            axes: Axes {
                pitch,
                roll: AxisCapabilities::UNSUPPORTED,
                yaw: AxisCapabilities::UNSUPPORTED,
            },
            recenter: Capability::Unsupported,
            stop: StopGuarantee::BestEffort,
        },
        zoom: Capability::Unsupported,
    }
}
