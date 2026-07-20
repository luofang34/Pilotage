//! Folding decoded MAVLink messages into the shared link cache:
//! source filtering, liveness, estimator authorization, and the
//! measurement-group acquisition discipline.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::estimator::{accept_status, authorization_at, invalidate_cached_authorization};
use super::measurement::{next_attitude_stamp, next_kinematics_stamp};
use super::{
    AttitudeUpdate, AuthorizationSource, CommandAckReport, GimbalDeviceAttitude, KinematicsUpdate,
    LinkState, estimator,
};
use crate::codec::{FcMessage, FrameSource};

/// Folds decoded messages into the shared cache. Kept synchronous and
/// lock-scoped: the lock is never held across an await.
pub(super) fn apply_messages(
    state: &Arc<Mutex<LinkState>>,
    messages: &[(FrameSource, FcMessage)],
    crc_failures: u32,
    unknown_ids: u32,
) {
    apply_messages_at(state, messages, crc_failures, unknown_ids, Instant::now());
}

/// Applies a standard ESTIMATOR_STATUS (msg 230): standard-status
/// dialects authorize from it; the Aviate dialect treats it as
/// diagnostic only.
fn apply_standard_status(latest: &mut LinkState, time_usec: u64, flags: u16, now: Instant) {
    if latest.authorization_source == AuthorizationSource::StandardEstimatorStatus {
        let (valid_flags, quality) = estimator::standard_authorization(flags);
        let aligned_usec = (time_usec / 1_000) * 1_000;
        accept_status(latest, aligned_usec, valid_flags, quality, now);
    }
}

/// MAV_RESULT_ACCEPTED = 0. A refused command must be loud: a denied
/// disarm or mode change looks exactly like an unresponsive vehicle
/// otherwise.
fn note_command_ack(command: u16, result: u8) {
    if result != 0 {
        tracing::warn!(command, result, "FC refused a command");
    }
}

/// Caches one command acknowledgement addressed to this GCS identity.
/// Acks for another endpoint prove nothing about our commands; zero
/// targets mean the sender omitted the v2 extension fields (accepted as
/// unaddressed). The gimbal CONFIGURE verdict is tracked apart from the
/// general slot so a later, unrelated ack (e.g. periodic
/// SET_MESSAGE_INTERVAL) cannot bury a claim denial.
fn apply_command_ack(
    latest: &mut LinkState,
    command: u16,
    result: u8,
    target_system: u8,
    target_component: u8,
    now: Instant,
) {
    let addressed_to_us = (target_system == 0 && target_component == 0)
        || (target_system == crate::codec::GCS_SYSTEM_ID
            && target_component == crate::codec::GCS_COMPONENT_ID);
    if !addressed_to_us {
        return;
    }
    note_command_ack(command, result);
    let report = CommandAckReport {
        command,
        result,
        received_at: now,
    };
    latest.last_command_ack = Some(report);
    if command == crate::codec::MAV_CMD_DO_GIMBAL_MANAGER_CONFIGURE {
        latest.gimbal_configure_ack = Some(report);
    }
}

/// Folds decoded messages into the shared cache at an explicit receive
/// instant. Public so adapter crates can drive the cache in tests
/// without a socket; production traffic arrives via the link task.
pub fn apply_messages_at(
    state: &Arc<Mutex<LinkState>>,
    messages: &[(FrameSource, FcMessage)],
    crc_failures: u32,
    unknown_ids: u32,
    now: Instant,
) {
    let Ok(mut latest) = state.lock() else {
        return;
    };
    latest.crc_failures = latest.crc_failures.wrapping_add(u64::from(crc_failures));
    latest.unknown_ids = latest.unknown_ids.wrapping_add(u64::from(unknown_ids));
    for &(source, message) in messages {
        if source.system_id != latest.system_id || source.component_id != latest.component_id {
            latest.wrong_sources = latest.wrong_sources.wrapping_add(1);
            continue;
        }
        if message == FcMessage::InvalidAviateEstimatorStatus {
            latest.invalid_estimator_statuses = latest.invalid_estimator_statuses.wrapping_add(1);
            invalidate_cached_authorization(&mut latest);
            continue;
        }
        latest.decoded = latest.decoded.wrapping_add(1);
        apply_message(&mut latest, message, now);
    }
}

/// Folds one attitude group into the cache, stamping it with the
/// authorization current at its source time.
fn apply_attitude(
    latest: &mut LinkState,
    time_boot_ms: u32,
    quat_wxyz: [f32; 4],
    rates_rps: [f32; 3],
    now: Instant,
) {
    if let Some(stamp) = next_attitude_stamp(latest, time_boot_ms, now) {
        let authorization = authorization_at(latest, time_boot_ms);
        latest.attitude = Some(AttitudeUpdate {
            quat_wxyz,
            rates_rps,
            time_boot_ms,
            stamp,
            valid_flags: authorization.valid_flags,
            quality: authorization.quality,
            received_at: now,
        });
    }
}

/// Folds one kinematics group into the cache, stamping it with the
/// authorization current at its source time.
fn apply_kinematics(
    latest: &mut LinkState,
    time_boot_ms: u32,
    pos_ned_m: [f32; 3],
    vel_ned_mps: [f32; 3],
    now: Instant,
) {
    if let Some(stamp) = next_kinematics_stamp(latest, time_boot_ms, now) {
        let authorization = authorization_at(latest, time_boot_ms);
        latest.kinematics = Some(KinematicsUpdate {
            pos_ned_m,
            vel_ned_mps,
            time_boot_ms,
            stamp,
            valid_flags: authorization.valid_flags,
            quality: authorization.quality,
            received_at: now,
        });
    }
}

/// Applies one source-matched decoded message to the cache.
fn apply_message(latest: &mut LinkState, message: FcMessage, now: Instant) {
    match message {
        FcMessage::InvalidAviateEstimatorStatus => {}
        FcMessage::Heartbeat { armed } => {
            latest.last_heartbeat = Some(now);
            latest.heartbeat_armed = Some(armed);
        }
        FcMessage::CommandAck {
            command,
            result,
            target_system,
            target_component,
        } => apply_command_ack(
            latest,
            command,
            result,
            target_system,
            target_component,
            now,
        ),
        FcMessage::EstimatorStatus { time_usec, flags } => {
            apply_standard_status(latest, time_usec, flags, now);
        }
        FcMessage::AviateEstimatorStatus {
            time_usec,
            valid_flags,
            quality,
        } => accept_status(latest, time_usec, valid_flags, quality, now),
        FcMessage::AttitudeQuaternion {
            time_boot_ms,
            quat_wxyz,
            rates_rps,
        } => apply_attitude(latest, time_boot_ms, quat_wxyz, rates_rps, now),
        FcMessage::LocalPositionNed {
            time_boot_ms,
            pos_ned_m,
            vel_ned_mps,
        } => apply_kinematics(latest, time_boot_ms, pos_ned_m, vel_ned_mps, now),
        FcMessage::GimbalDeviceAttitudeStatus {
            time_boot_ms,
            quat_wxyz,
            rates_rps,
            flags,
            failure_flags,
        } => {
            if failure_flags != 0 {
                tracing::warn!(failure_flags, "gimbal device reports a failure condition");
            }
            latest.gimbal_device = Some(GimbalDeviceAttitude {
                quat_wxyz,
                rates_rps,
                time_boot_ms,
                flags,
                failure_flags,
                received_at: now,
            });
        }
    }
}
