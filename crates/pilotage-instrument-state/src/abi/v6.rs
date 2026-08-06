//! Tagged-group state ABI, version 6 (ADR-0029 extensible state groups).
//!
//! The frame is self-delimiting:
//!
//! ```text
//! [0] u8 version (= 6)
//! [1] u8 group count (N)
//! then N groups, each:
//!     u8  group id        (strictly ascending across the frame)
//!     u16 payload length  (LE)
//!     payload
//! ```
//!
//! Presence is meaning: an absent tag IS the group never fed, so an
//! unfed group resolves `Missing` by construction — no producer opt-in.
//! A source with a different group set simply writes different tags.
//!
//! Forward compatibility mirrors the scene-opcode policy: an unknown tag
//! is a counted skip, and a known group's payload may grow by appending
//! fields — a longer payload is accepted with the tail counted, a
//! shorter-than-minimum one fails that group. The encoder emits present
//! groups in strictly ascending id order, so equal states produce equal
//! bytes ([`fixtures`] pins that against committed golden frames).
//!
//! Payload layouts live with their codecs in [`stamped`] and
//! [`declared`]. Field codings: NaN encodes an absent optional float;
//! enum bytes count from zero in declaration order with 255 as the
//! fail-closed unknown, and a wire value outside the known set decodes
//! to each type's `Unknown`, never to a benign variant (VAL-01).

use crate::aircraft::AircraftState;
use crate::group_id::GroupId;

pub mod fixtures;

mod declared;
mod monitor;
mod stamped;

/// Version stamped in the frame's first byte.
pub const VERSION: u8 = 6;

/// Buffer capacity a feeder allocates. This is an allocation bound, not
/// wire shape: the frame is self-delimiting, and growing the capacity is
/// not a wire break because consumers read it at runtime.
pub const CAPACITY: usize = 1024;

/// Why a v6 frame failed to encode or decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AbiError {
    /// The buffer ends before the announced content does.
    #[error("frame truncated")]
    Truncated,
    /// The version byte is one this codec does not read.
    #[error("state ABI version {found} is not {VERSION}")]
    BadVersion {
        /// The version found.
        found: u8,
    },
    /// A tag repeated or descended: the frame is not canonical.
    #[error("group tag {id:#04x} out of canonical ascending order")]
    NonCanonicalOrder {
        /// The offending tag.
        id: u8,
    },
    /// A known group's payload is shorter than its minimum layout.
    #[error("group {id:?} payload below its minimum length")]
    GroupTruncated {
        /// The truncated group.
        id: GroupId,
    },
}

/// A decoded frame plus the forward-compatibility counters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecodeReport {
    /// The decoded state; groups whose tags were absent are default —
    /// data `None`, ages `None`, trust at its fail-closed defaults.
    pub state: AircraftState,
    /// Tags this build cannot place, counted and skipped.
    pub unknown_groups: u8,
    /// Known groups whose payload carried an ignored appended tail.
    pub extended_groups: u8,
}

/// Minimum payload bytes for each group this build knows.
const fn min_len(id: GroupId) -> usize {
    match id {
        GroupId::Attitude => 32,
        GroupId::Kinematics => 28,
        GroupId::Air => 12,
        GroupId::Nav => 42,
        GroupId::Wind => 12,
        GroupId::Selections => 20,
        GroupId::Trust => 8,
        GroupId::Altitude => 12,
        GroupId::Heading => 12,
        GroupId::Variation => 12,
        GroupId::Dynamics => 16,
        GroupId::MonitorText => monitor::MONITOR_LEN,
    }
}

/// Decodes one group payload into `state` (exhaustive: a new group
/// variant fails to compile until it decodes).
fn decode_group(state: &mut AircraftState, id: GroupId, payload: &[u8]) {
    match id {
        GroupId::Attitude => stamped::decode_attitude(state, payload),
        GroupId::Kinematics => stamped::decode_kinematics(state, payload),
        GroupId::Air => stamped::decode_air(state, payload),
        GroupId::Nav => stamped::decode_nav(state, payload),
        GroupId::Wind => stamped::decode_wind(state, payload),
        GroupId::Selections => declared::decode_selections(state, payload),
        GroupId::Trust => declared::decode_trust(state, payload),
        GroupId::Altitude => declared::decode_altitude(state, payload),
        GroupId::Heading => stamped::decode_heading(state, payload),
        GroupId::Variation => stamped::decode_variation(state, payload),
        GroupId::Dynamics => stamped::decode_dynamics(state, payload),
        GroupId::MonitorText => monitor::decode_monitor_text(state, payload),
    }
}

/// Encodes one group's payload into `out`; `Ok(None)` when the group is
/// absent from `state` and its tag must be omitted (exhaustive: a new
/// group variant fails to compile until it encodes).
fn encode_group(
    state: &AircraftState,
    id: GroupId,
    out: &mut [u8],
) -> Result<Option<usize>, AbiError> {
    match id {
        GroupId::Attitude => stamped::encode_attitude(state, out),
        GroupId::Kinematics => stamped::encode_kinematics(state, out),
        GroupId::Air => stamped::encode_air(state, out),
        GroupId::Nav => stamped::encode_nav(state, out),
        GroupId::Wind => stamped::encode_wind(state, out),
        GroupId::Selections => declared::encode_selections(state, out),
        GroupId::Trust => declared::encode_trust(state, out),
        GroupId::Altitude => declared::encode_altitude(state, out),
        GroupId::Heading => stamped::encode_heading(state, out),
        GroupId::Variation => stamped::encode_variation(state, out),
        GroupId::Dynamics => stamped::encode_dynamics(state, out),
        GroupId::MonitorText => monitor::encode_monitor_text(state, out),
    }
}

/// Decodes a v6 frame.
pub fn decode_state(buf: &[u8]) -> Result<DecodeReport, AbiError> {
    let version = *buf.first().ok_or(AbiError::Truncated)?;
    if version != VERSION {
        return Err(AbiError::BadVersion { found: version });
    }
    let count = *buf.get(1).ok_or(AbiError::Truncated)?;

    let mut report = DecodeReport {
        state: AircraftState::default(),
        unknown_groups: 0,
        extended_groups: 0,
    };
    let mut offset = 2usize;
    let mut prev_tag = 0u8;
    for _ in 0..count {
        let header = buf.get(offset..offset + 3).ok_or(AbiError::Truncated)?;
        let tag = header[0];
        let len = u16::from_le_bytes([header[1], header[2]]) as usize;
        if tag <= prev_tag {
            return Err(AbiError::NonCanonicalOrder { id: tag });
        }
        prev_tag = tag;
        let payload = buf
            .get(offset + 3..offset + 3 + len)
            .ok_or(AbiError::Truncated)?;
        match GroupId::from_u8(tag) {
            Some(id) => {
                if len < min_len(id) {
                    return Err(AbiError::GroupTruncated { id });
                }
                if len > min_len(id) {
                    report.extended_groups = report.extended_groups.saturating_add(1);
                }
                decode_group(&mut report.state, id, payload);
            }
            None => {
                report.unknown_groups = report.unknown_groups.saturating_add(1);
            }
        }
        offset += 3 + len;
    }
    Ok(report)
}

/// Encodes `state` as a canonical v6 frame — present groups only, in
/// ascending tag order — returning the used length.
pub fn encode_state(state: &AircraftState, buf: &mut [u8]) -> Result<usize, AbiError> {
    if buf.len() < 2 {
        return Err(AbiError::Truncated);
    }
    buf[0] = VERSION;
    let mut count = 0u8;
    let mut offset = 2usize;
    for id in GroupId::ALL {
        let body = buf.get_mut(offset + 3..).ok_or(AbiError::Truncated)?;
        let Some(len) = encode_group(state, id, body)? else {
            continue;
        };
        let len16 = u16::try_from(len).map_err(|_| AbiError::Truncated)?;
        let header = buf.get_mut(offset..offset + 3).ok_or(AbiError::Truncated)?;
        header[0] = id.to_u8();
        header[1..3].copy_from_slice(&len16.to_le_bytes());
        count = count.saturating_add(1);
        offset += 3 + len;
    }
    buf[1] = count;
    Ok(offset)
}

// Payload-local field access. Bounds are structurally guaranteed by the
// minimum-length check before dispatch and the size check each encoder
// performs, so misses are logic errors; reads fail safe (NaN reads as
// absent, 255 as the fail-closed unknown enum byte) and writes to an
// impossible range are dropped rather than panicking.

pub(super) fn get_f32(payload: &[u8], off: usize) -> f32 {
    payload
        .get(off..off + 4)
        .map_or(f32::NAN, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub(super) fn get_u8(payload: &[u8], off: usize) -> u8 {
    payload.get(off).copied().unwrap_or(255)
}

pub(super) fn get_u16(payload: &[u8], off: usize) -> u16 {
    payload
        .get(off..off + 2)
        .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
}

pub(super) fn get_u32(payload: &[u8], off: usize) -> u32 {
    payload
        .get(off..off + 4)
        .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub(super) fn put_f32(payload: &mut [u8], off: usize, value: f32) {
    if let Some(b) = payload.get_mut(off..off + 4) {
        b.copy_from_slice(&value.to_le_bytes());
    }
}

pub(super) fn put_u8(payload: &mut [u8], off: usize, value: u8) {
    if let Some(b) = payload.get_mut(off) {
        *b = value;
    }
}

pub(super) fn put_u16(payload: &mut [u8], off: usize, value: u16) {
    if let Some(b) = payload.get_mut(off..off + 2) {
        b.copy_from_slice(&value.to_le_bytes());
    }
}

pub(super) fn put_u32(payload: &mut [u8], off: usize, value: u32) {
    if let Some(b) = payload.get_mut(off..off + 4) {
        b.copy_from_slice(&value.to_le_bytes());
    }
}

#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod posture_tests;
#[cfg(test)]
mod tests;
