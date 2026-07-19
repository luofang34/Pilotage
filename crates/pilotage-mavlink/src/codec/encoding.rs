//! Frame serialization for the uplink subset: GCS presence, commands,
//! offboard setpoints, and gimbal-manager demands.

use super::{
    COMMAND_LONG_ID, GIMBAL_MANAGER_SET_ATTITUDE_ID, HEARTBEAT_ID, MAGIC_V2,
    SET_POSITION_TARGET_ID, compute_crc,
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
fn encode_frame_v2(seq: u8, msg_id: u32, payload: &[u8], extra: u8, out: &mut [u8]) -> usize {
    let total = 10 + payload.len() + 2;
    if out.len() < total || payload.len() > 255 {
        return 0;
    }
    out[0] = MAGIC_V2;
    out[1] = payload.len() as u8;
    out[2] = 0;
    out[3] = 0;
    out[4] = seq;
    out[5] = GCS_SYSTEM_ID;
    out[6] = GCS_COMPONENT_ID;
    out[7] = (msg_id & 0xff) as u8;
    out[8] = ((msg_id >> 8) & 0xff) as u8;
    out[9] = ((msg_id >> 16) & 0xff) as u8;
    out[10..10 + payload.len()].copy_from_slice(payload);
    let crc = compute_crc(&out[1..10 + payload.len()], extra);
    out[10 + payload.len()] = (crc & 0xff) as u8;
    out[10 + payload.len() + 1] = (crc >> 8) as u8;
    total
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

/// Absolute FPV attitude demand and the component selected to receive it.
///
/// Body-rate demand is intentionally absent because the FC attitude loop
/// derives rate commands from this target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttitudeTarget {
    /// Roll demand in radians.
    pub roll_rad: f32,
    /// Pitch demand in radians.
    pub pitch_rad: f32,
    /// Yaw demand in radians.
    pub yaw_rad: f32,
    /// Normalized collective thrust.
    pub thrust: f32,
    /// Selected MAVLink system id.
    pub system_id: u8,
    /// Selected MAVLink component id.
    pub component_id: u8,
}

/// Encodes one absolute attitude target for the selected component.
pub fn encode_attitude_setpoint(seq: u8, time_boot_ms: u32, target: AttitudeTarget) -> [u8; 51] {
    const SET_ATTITUDE_TARGET_ID: u32 = 82;
    let (sr, cr) = (target.roll_rad * 0.5).sin_cos();
    let (sp, cp) = (target.pitch_rad * 0.5).sin_cos();
    let (sy, cy) = (target.yaw_rad * 0.5).sin_cos();
    let q = [
        cr * cp * cy + sr * sp * sy,
        sr * cp * cy - cr * sp * sy,
        cr * sp * cy + sr * cp * sy,
        cr * cp * sy - sr * sp * cy,
    ];
    let mut payload = [0u8; 39];
    payload[0..4].copy_from_slice(&time_boot_ms.to_le_bytes());
    for (i, w) in q.iter().enumerate() {
        payload[4 + i * 4..8 + i * 4].copy_from_slice(&w.to_le_bytes());
    }
    // body rates [20..32) stay zero (masked); thrust at [32..36).
    payload[32..36].copy_from_slice(&target.thrust.clamp(0.0, 1.0).to_le_bytes());
    payload[36] = target.system_id;
    payload[37] = target.component_id;
    payload[38] = 0b0000_0111; // ignore body roll/pitch/yaw rate
    let mut frame = [0u8; 51];
    encode_frame_v2(seq, SET_ATTITUDE_TARGET_ID, &payload, 49, &mut frame);
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
/// velocity demand in rad/s under [`GIMBAL_FLAGS_HORIZON_YAW_FOLLOW`].
///
/// PX4 silently drops this message unless the sender holds gimbal
/// primary control, so a streaming caller must pair it with periodic
/// DO_GIMBAL_MANAGER_CONFIGURE claims naming
/// [`GCS_SYSTEM_ID`]/[`GCS_COMPONENT_ID`].
///
/// Wire order (35 bytes): flags u32 @0, q\[4\] @4..20, angular_velocity
/// x/y/z @20..32, target_system @32, target_component @33,
/// gimbal_device_id @34 (0 = all gimbal devices).
pub fn encode_gimbal_rate_setpoint(
    seq: u8,
    pitch_rate_rps: f32,
    yaw_rate_rps: f32,
    target_system: u8,
    target_component: u8,
) -> [u8; 47] {
    let mut payload = [0u8; 35];
    payload[0..4].copy_from_slice(&GIMBAL_FLAGS_HORIZON_YAW_FOLLOW.to_le_bytes());
    for slot in 0..5 {
        // q[0..4] and angular_velocity_x: NaN = ignored.
        let at = 4 + slot * 4;
        payload[at..at + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    }
    payload[24..28].copy_from_slice(&pitch_rate_rps.to_le_bytes());
    payload[28..32].copy_from_slice(&yaw_rate_rps.to_le_bytes());
    payload[32] = target_system;
    payload[33] = target_component;
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
