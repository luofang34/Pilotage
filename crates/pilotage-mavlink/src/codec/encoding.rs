//! Frame serialization for the uplink subset: GCS presence, commands,
//! offboard setpoints, and gimbal-manager demands.

use super::{
    COMMAND_ACK_ID, COMMAND_LONG_ID, GIMBAL_MANAGER_SET_ATTITUDE_ID, HEARTBEAT_ID, MAGIC_V2,
    MANUAL_CONTROL_ID, SET_POSITION_TARGET_ID, compute_crc,
};

/// MAVLink system id stamped on every frame this codec encodes (the
/// conventional GCS identity). Gimbal primary-control claims must name
/// this identity or PX4 silently ignores the subsequent demands.
pub const GCS_SYSTEM_ID: u8 = 255;
/// MAVLink component id stamped on every frame this codec encodes.
pub const GCS_COMPONENT_ID: u8 = 190;

/// Writes a MAVLink 2.0 frame ([`GCS_SYSTEM_ID`]/[`GCS_COMPONENT_ID`])
/// around `payload` into `out`, returning the frame length. `out` must
/// hold `10 + payload.len() + 2` bytes; a too-small buffer returns 0.
pub(super) fn encode_frame_v2(
    seq: u8,
    msg_id: u32,
    payload: &[u8],
    extra: u8,
    out: &mut [u8],
) -> usize {
    encode_frame_v2_from(
        seq,
        msg_id,
        payload,
        extra,
        GCS_SYSTEM_ID,
        GCS_COMPONENT_ID,
        out,
    )
}

/// Writes a MAVLink 2.0 frame under an explicit SOURCE identity — the
/// encoder for messages another party originates (an autopilot, a fake
/// FC in a test, a gateway loopback, a simulator). `out` must hold
/// `10 + payload.len() + 2` bytes; a too-small buffer returns 0.
#[allow(clippy::too_many_arguments)]
pub fn encode_frame_v2_from(
    seq: u8,
    msg_id: u32,
    payload: &[u8],
    extra: u8,
    system_id: u8,
    component_id: u8,
    out: &mut [u8],
) -> usize {
    let total = 10 + payload.len() + 2;
    if out.len() < total || payload.len() > 255 {
        return 0;
    }
    out[0] = MAGIC_V2;
    out[1] = payload.len() as u8;
    out[2] = 0;
    out[3] = 0;
    out[4] = seq;
    out[5] = system_id;
    out[6] = component_id;
    out[7] = (msg_id & 0xff) as u8;
    out[8] = ((msg_id >> 8) & 0xff) as u8;
    out[9] = ((msg_id >> 16) & 0xff) as u8;
    out[10..10 + payload.len()].copy_from_slice(payload);
    let crc = compute_crc(&out[1..10 + payload.len()], extra);
    out[10 + payload.len()] = (crc & 0xff) as u8;
    out[10 + payload.len() + 1] = (crc >> 8) as u8;
    total
}

/// Serializes an FC-sourced COMMAND_ACK frame addressed to the GCS
/// identity this codec commands under — the acknowledgement an autopilot
/// returns for a COMMAND_LONG (fake FCs in tests, gateway loopback).
///
/// Wire order (10 bytes): command u16 @0, result u8 @2, progress u8 @3,
/// result_param2 i32 @4, target_system @8, target_component @9.
pub fn encode_command_ack(
    seq: u8,
    command: u16,
    result: u8,
    system_id: u8,
    component_id: u8,
) -> [u8; 22] {
    let mut payload = [0u8; 10];
    payload[0..2].copy_from_slice(&command.to_le_bytes());
    payload[2] = result;
    payload[8] = GCS_SYSTEM_ID;
    payload[9] = GCS_COMPONENT_ID;
    let mut frame = [0u8; 22];
    encode_frame_v2_from(
        seq,
        COMMAND_ACK_ID,
        &payload,
        143,
        system_id,
        component_id,
        &mut frame,
    );
    frame
}

/// Serializes a GCS heartbeat frame, used to register this endpoint with
/// a MAVLink router that only forwards to peers it has heard from.
pub fn encode_gcs_heartbeat(seq: u8) -> [u8; 21] {
    // Payload: custom_mode u32 (0), type MAV_TYPE_GCS=6, autopilot
    // MAV_AUTOPILOT_INVALID=8, base_mode 0, system_status MAV_STATE_ACTIVE=4,
    // mavlink_version 3.
    let mut payload = [0u8; 9];
    payload[4] = 6;
    payload[5] = 8;
    payload[7] = 4;
    payload[8] = 3;
    let mut frame = [0u8; 21];
    encode_frame_v2(seq, HEARTBEAT_ID, &payload, 50, &mut frame);
    frame
}

/// Serializes a COMMAND_LONG arm/disarm frame (MAV_CMD 400) for the selected
/// MAVLink system and component.
///
/// Wire order (33 bytes): param1..7 f32 @0..28, command u16 @28,
/// target_system @30, target_component @31, confirmation @32.
pub fn encode_arm_command(seq: u8, arm: bool, target_system: u8, target_component: u8) -> [u8; 45] {
    let mut payload = [0u8; 33];
    let param1: f32 = if arm { 1.0 } else { 0.0 };
    payload[0..4].copy_from_slice(&param1.to_le_bytes());
    payload[28..30].copy_from_slice(&400u16.to_le_bytes());
    payload[30] = target_system;
    payload[31] = target_component;
    let mut frame = [0u8; 45];
    encode_frame_v2(seq, COMMAND_LONG_ID, &payload, 152, &mut frame);
    frame
}

/// Serializes an arbitrary COMMAND_LONG frame (mode changes, message
/// interval requests, and any other MAV_CMD the arm helper does not
/// cover) for the selected MAVLink system and component.
///
/// Wire order (33 bytes): param1..7 f32 @0..28, command u16 @28,
/// target_system @30, target_component @31, confirmation @32.
pub fn encode_command_long(
    seq: u8,
    command: u16,
    params: [f32; 7],
    target_system: u8,
    target_component: u8,
) -> [u8; 45] {
    let mut payload = [0u8; 33];
    for (index, param) in params.iter().enumerate() {
        let at = index * 4;
        payload[at..at + 4].copy_from_slice(&param.to_le_bytes());
    }
    payload[28..30].copy_from_slice(&command.to_le_bytes());
    payload[30] = target_system;
    payload[31] = target_component;
    let mut frame = [0u8; 45];
    encode_frame_v2(seq, COMMAND_LONG_ID, &payload, 152, &mut frame);
    frame
}

/// Serializes a SET_POSITION_TARGET_LOCAL_NED **position-hold**
/// setpoint: NED position plus absolute yaw, velocity/acceleration/yaw
/// rate masked out (the combination Aviate's bridge maps to
/// `ControlMode::PositionHold`) — DJI's brake-then-hold on centered
/// sticks.
pub fn encode_position_setpoint(
    seq: u8,
    time_boot_ms: u32,
    pos_ned_m: [f32; 3],
    yaw_rad: f32,
    target_system: u8,
    target_component: u8,
) -> [u8; 65] {
    // Ignore velocity (8|16|32), acceleration (64|128|256), yaw rate
    // (2048): position + absolute yaw remain.
    const TYPE_MASK: u16 = 8 | 16 | 32 | 64 | 128 | 256 | 2048;
    let mut payload = [0u8; 53];
    payload[0..4].copy_from_slice(&time_boot_ms.to_le_bytes());
    payload[4..8].copy_from_slice(&pos_ned_m[0].to_le_bytes());
    payload[8..12].copy_from_slice(&pos_ned_m[1].to_le_bytes());
    payload[12..16].copy_from_slice(&pos_ned_m[2].to_le_bytes());
    payload[40..44].copy_from_slice(&yaw_rad.to_le_bytes());
    payload[48..50].copy_from_slice(&TYPE_MASK.to_le_bytes());
    payload[50] = target_system;
    payload[51] = target_component;
    payload[52] = 1; // MAV_FRAME_LOCAL_NED
    let mut frame = [0u8; 65];
    encode_frame_v2(seq, SET_POSITION_TARGET_ID, &payload, 143, &mut frame);
    frame
}

/// Serializes a SET_POSITION_TARGET_LOCAL_NED velocity setpoint: NED
/// velocity plus absolute yaw, everything else masked out
/// (`type_mask` ignores position, acceleration, and yaw rate — the
/// combination Aviate's bridge maps to `ControlMode::VelocityControl`).
///
/// Wire order (53 bytes): time_boot_ms u32 @0, x/y/z @4..16,
/// vx/vy/vz @16..28, afx/afy/afz @28..40, yaw @40, yaw_rate @44,
/// type_mask u16 @48, target_system @50, target_component @51,
/// coordinate_frame @52.
pub fn encode_velocity_setpoint(
    seq: u8,
    time_boot_ms: u32,
    vel_ned_mps: [f32; 3],
    yaw_rad: f32,
    target_system: u8,
    target_component: u8,
) -> [u8; 65] {
    // Ignore position (1|2|4), acceleration (64|128|256), yaw rate (2048):
    // velocity + absolute yaw remain.
    const TYPE_MASK: u16 = 1 | 2 | 4 | 64 | 128 | 256 | 2048;
    let mut payload = [0u8; 53];
    payload[0..4].copy_from_slice(&time_boot_ms.to_le_bytes());
    payload[16..20].copy_from_slice(&vel_ned_mps[0].to_le_bytes());
    payload[20..24].copy_from_slice(&vel_ned_mps[1].to_le_bytes());
    payload[24..28].copy_from_slice(&vel_ned_mps[2].to_le_bytes());
    payload[40..44].copy_from_slice(&yaw_rad.to_le_bytes());
    payload[48..50].copy_from_slice(&TYPE_MASK.to_le_bytes());
    payload[50] = target_system;
    payload[51] = target_component;
    payload[52] = 1; // MAV_FRAME_LOCAL_NED
    let mut frame = [0u8; 65];
    encode_frame_v2(seq, SET_POSITION_TARGET_ID, &payload, 143, &mut frame);
    frame
}

/// MAV_FRAME_BODY_OFFSET_NED: axes aligned with the vehicle heading
/// (x forward, y right, z down), level regardless of roll and pitch.
pub const MAV_FRAME_BODY_OFFSET_NED: u8 = 9;

/// Serializes a SET_POSITION_TARGET_LOCAL_NED body-frame velocity and
/// yaw-rate setpoint in [`MAV_FRAME_BODY_OFFSET_NED`]: position,
/// acceleration, and absolute yaw are masked out. The FC rotates the
/// velocity by its own heading, so the caller needs no attitude to fly a
/// body-frame demand.
///
/// Wire order as [`encode_velocity_setpoint`].
pub fn encode_body_velocity_setpoint(
    seq: u8,
    time_boot_ms: u32,
    vel_frd_mps: [f32; 3],
    yaw_rate_rps: f32,
    target_system: u8,
    target_component: u8,
) -> [u8; 65] {
    // Ignore position (1|2|4), acceleration (64|128|256), yaw (1024):
    // velocity + yaw rate remain.
    const TYPE_MASK: u16 = 1 | 2 | 4 | 64 | 128 | 256 | 1024;
    let mut payload = [0u8; 53];
    payload[0..4].copy_from_slice(&time_boot_ms.to_le_bytes());
    payload[16..20].copy_from_slice(&vel_frd_mps[0].to_le_bytes());
    payload[20..24].copy_from_slice(&vel_frd_mps[1].to_le_bytes());
    payload[24..28].copy_from_slice(&vel_frd_mps[2].to_le_bytes());
    payload[44..48].copy_from_slice(&yaw_rate_rps.to_le_bytes());
    payload[48..50].copy_from_slice(&TYPE_MASK.to_le_bytes());
    payload[50] = target_system;
    payload[51] = target_component;
    payload[52] = MAV_FRAME_BODY_OFFSET_NED;
    let mut frame = [0u8; 65];
    encode_frame_v2(seq, SET_POSITION_TARGET_ID, &payload, 143, &mut frame);
    frame
}

/// GIMBAL_MANAGER_FLAGS with roll and pitch locked to the horizon and
/// yaw following the vehicle (no YAW_LOCK): the operator-camera
/// default, so the view pans with the airframe and holds level through
/// attitude changes.
pub const GIMBAL_FLAGS_HORIZON_YAW_FOLLOW: u32 = 4 | 8;

/// Serializes a GIMBAL_MANAGER_SET_ATTITUDE rate demand: the attitude
/// quaternion and roll rate are NaN ("ignored" per the message spec;
/// the device keeps its own horizon), leaving the pitch/yaw angular
/// velocity demand in rad/s under the caller's validated reference flags.
///
/// The caller must confirm ownership from the selected manager's status.
///
/// Wire order (35 bytes): flags u32 @0, q\[4\] @4..20, angular_velocity
/// x/y/z @20..32, target_system @32, target_component @33,
/// gimbal_device_id @34 (0 = all gimbal devices).
pub fn encode_gimbal_rate_setpoint(
    seq: u8,
    flags: u32,
    rates_rps: [f32; 3],
    target_system: u8,
    target_component: u8,
    gimbal_device_id: u8,
) -> [u8; 47] {
    let mut payload = [0u8; 35];
    payload[0..4].copy_from_slice(&flags.to_le_bytes());
    for slot in 0..4 {
        // The quaternion is omitted because this packet requests rates only.
        let at = 4 + slot * 4;
        payload[at..at + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    }
    for (index, rate) in rates_rps.iter().enumerate() {
        let offset = 20 + index * 4;
        payload[offset..offset + 4].copy_from_slice(&rate.to_le_bytes());
    }
    payload[32] = target_system;
    payload[33] = target_component;
    payload[34] = gimbal_device_id;
    let mut frame = [0u8; 47];
    encode_frame_v2(
        seq,
        GIMBAL_MANAGER_SET_ATTITUDE_ID,
        &payload,
        123,
        &mut frame,
    );
    frame
}

/// One MANUAL_CONTROL stick sample in the wire's own units: pitch `x`,
/// roll `y`, and yaw `r` in `[-1000, 1000]`, throttle `z` in `[0, 1000]`.
/// Out-of-range values are clamped so the FC never discards the sample as
/// invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManualControlSticks {
    /// Pitch stick; + is stick forward.
    pub x: i16,
    /// Roll stick; + is stick right.
    pub y: i16,
    /// Throttle stick; 500 is centered.
    pub z: i16,
    /// Yaw stick; + is stick right.
    pub r: i16,
}

/// Serializes a MANUAL_CONTROL frame (the MAVLink joystick) addressed to
/// `target_system`, with no buttons and no extension fields.
///
/// Wire order (11 bytes): x/y/z/r i16 @0..8, buttons u16 @8, target @10.
#[must_use]
pub fn encode_manual_control(seq: u8, sticks: ManualControlSticks, target_system: u8) -> [u8; 23] {
    let mut payload = [0u8; 11];
    payload[0..2].copy_from_slice(&sticks.x.clamp(-1000, 1000).to_le_bytes());
    payload[2..4].copy_from_slice(&sticks.y.clamp(-1000, 1000).to_le_bytes());
    payload[4..6].copy_from_slice(&sticks.z.clamp(0, 1000).to_le_bytes());
    payload[6..8].copy_from_slice(&sticks.r.clamp(-1000, 1000).to_le_bytes());
    payload[10] = target_system;
    let mut frame = [0u8; 23];
    encode_frame_v2(seq, MANUAL_CONTROL_ID, &payload, 243, &mut frame);
    frame
}

/// Serializes an FC-sourced HEARTBEAT reporting the armed flag and an
/// autopilot custom mode, for fake FCs in tests.
#[must_use]
pub fn encode_autopilot_heartbeat(
    seq: u8,
    armed: bool,
    custom_mode: u32,
    system_id: u8,
    component_id: u8,
) -> [u8; 21] {
    // custom_mode u32 @0, type @4 (MAV_TYPE_GENERIC), autopilot @5
    // (MAV_AUTOPILOT_PX4 = 12), base_mode @6 (CUSTOM_MODE_ENABLED, plus
    // SAFETY_ARMED when armed), system_status @7 (ACTIVE), version @8.
    let mut payload = [0u8; 9];
    payload[0..4].copy_from_slice(&custom_mode.to_le_bytes());
    payload[5] = 12;
    payload[6] = 0x01 | if armed { 0x80 } else { 0 };
    payload[7] = 4;
    payload[8] = 3;
    let mut frame = [0u8; 21];
    encode_frame_v2_from(
        seq,
        HEARTBEAT_ID,
        &payload,
        50,
        system_id,
        component_id,
        &mut frame,
    );
    frame
}
