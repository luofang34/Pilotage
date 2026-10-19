use pilotage_protocol::wire;
use turret_core::{Command, CommandProgress, EndReason, PayloadCommand, Tick, ZoomCommand};

/// Preserve independent execution facts even when a command subsequently ends.
#[must_use]
pub fn payload_progress_to_wire(
    value: &CommandProgress,
    command: Option<&Command>,
    updated_at: Tick,
) -> wire::PayloadCommandProgress {
    let mut wire = wire::PayloadCommandProgress {
        command_id: value.identity.id,
        connection_epoch: value.identity.epoch,
        generation: value.identity.generation,
        sequence: value.identity.sequence,
        updated_at: super::timestamp(updated_at),
        admitted_at: super::timestamp(value.accepted_at),
        prepared_at: value.prepared_at.and_then(super::timestamp),
        sent_at: value.sent_at.and_then(super::timestamp),
        acknowledged_at: value.acknowledged_at.and_then(super::timestamp),
        acknowledgement_timed_out: value.acknowledgement_timed_out,
        observed_complete_at: value.observed_complete_at.and_then(super::timestamp),
        end_reason: value
            .ended
            .map_or(wire::PayloadEndReason::None as i32, end_reason),
        stage: stage(value) as i32,
        ..Default::default()
    };
    if let Some(command) = command.filter(|command| command.identity == value.identity) {
        target(&mut wire, command.payload);
    }
    wire
}

fn target(wire: &mut wire::PayloadCommandProgress, payload: PayloadCommand) {
    match payload {
        PayloadCommand::GimbalPosition(position) => {
            wire.frame = super::frame(position.frame);
            wire.pitch_target_rad = position.angles.pitch.map(|angle| angle.as_radians());
            wire.roll_target_rad = position.angles.roll.map(|angle| angle.as_radians());
            wire.yaw_target_rad = position.angles.yaw.map(|angle| angle.as_radians());
        }
        PayloadCommand::Zoom(ZoomCommand::Absolute(value)) => {
            let micros = f64::from(value.value()) * 1_000_000.0;
            if micros >= i64::MIN as f64 && micros < i64::MAX as f64 {
                wire.zoom_target = Some(wire::ZoomAbsoluteTarget {
                    unit: super::zoom_unit(value.unit()),
                    value_micros: micros.round() as i64,
                });
            }
        }
        PayloadCommand::GimbalRate(_) | PayloadCommand::Recenter | PayloadCommand::Zoom(_) => {}
    }
}

fn stage(value: &CommandProgress) -> wire::PayloadCommandStage {
    use wire::PayloadCommandStage as Stage;
    match value.ended {
        Some(EndReason::ObservedComplete) => Stage::ObservedComplete,
        Some(EndReason::ExpiredBeforeSend | EndReason::ValidityExpired) => Stage::Expired,
        Some(EndReason::DeviceRejected) => Stage::Rejected,
        Some(EndReason::Revoked | EndReason::Reconnected | EndReason::Superseded) => {
            Stage::Cancelled
        }
        Some(EndReason::CompletionTimeout | EndReason::TransportFailure) => Stage::OutcomeUnknown,
        None if value.observed_complete_at.is_some() => Stage::ObservedComplete,
        None if value.acknowledged_at.is_some() => Stage::DeviceAccepted,
        None if value.sent_at.is_some() => Stage::Sent,
        None if value.prepared_at.is_some() => Stage::Queued,
        None => Stage::Admitted,
    }
}

fn end_reason(value: EndReason) -> i32 {
    (match value {
        EndReason::ExpiredBeforeSend => wire::PayloadEndReason::ExpiredBeforeSend,
        EndReason::ValidityExpired => wire::PayloadEndReason::ValidityExpired,
        EndReason::CompletionTimeout => wire::PayloadEndReason::CompletionTimeout,
        EndReason::Revoked => wire::PayloadEndReason::Revoked,
        EndReason::Reconnected => wire::PayloadEndReason::Reconnected,
        EndReason::Superseded => wire::PayloadEndReason::Superseded,
        EndReason::DeviceRejected => wire::PayloadEndReason::DeviceRejected,
        EndReason::ObservedComplete => wire::PayloadEndReason::ObservedComplete,
        EndReason::TransportFailure => wire::PayloadEndReason::TransportFailure,
    }) as i32
}
