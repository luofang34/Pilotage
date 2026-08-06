//! Measurement identity stamps and the per-role legality rules (AV-01).
//!
//! Stamps arrive as the wire's own byte codings: the shell converts
//! JavaScript or FFI number shapes at its boundary, and every semantic
//! judgement — role match, clock-domain legality, integrity vocabulary,
//! wrap-safe serial ordering — happens here, once, for every shell.

/// Monotonic time since the producing vehicle computer booted.
pub const CLOCK_VEHICLE_BOOT: u8 = 1;
/// Monotonic simulation time supplied by the simulator.
pub const CLOCK_SIMULATION: u8 = 2;
/// Monotonic time on the receiving host, for wires carrying no source
/// timestamp.
pub const CLOCK_HOST_MONOTONIC: u8 = 3;

/// Estimator output: the only role primary panels admit.
pub const ROLE_OPERATIONAL_ESTIMATE: u8 = 1;
/// Simulator ground truth.
pub const ROLE_SIMULATION_TRUTH: u8 = 2;
/// Vehicle arm/mode/failsafe reports.
pub const ROLE_FC_STATE: u8 = 3;
/// Host navigation-component guidance (ADR-0031).
pub const ROLE_NAVIGATION_SOLUTION: u8 = 6;

const KNOWN_INTEGRITY: [u8; 3] = [1, 2, 3];
const SERIAL_HALF_RANGE: u32 = 0x8000_0000;

/// Identity and acquisition stamp for one measurement group, in the
/// wire's own codings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawStamp {
    /// Source role code; consumers gate on exact equality.
    pub role: u8,
    /// Integrity classification code of the delivering path.
    pub integrity: u8,
    /// Source identifier, stable within one vehicle and role.
    pub source_id: u64,
    /// Opaque attachment/boot identity.
    pub incarnation: [u8; 16],
    /// Source boot/attachment generation.
    pub epoch: u32,
    /// Wrapping group sequence, advanced only for a new measurement.
    pub sequence: u32,
    /// Acquisition time in nanoseconds in [`Self::clock`].
    pub acquired_at_ns: u64,
    /// Clock domain code for [`Self::acquired_at_ns`].
    pub clock: u8,
}

impl RawStamp {
    /// Whether two stamps carry the same source identity and clock —
    /// the precondition for any cross-stamp time comparison.
    pub fn same_stream(&self, other: &RawStamp) -> bool {
        self.source_id == other.source_id
            && self.incarnation == other.incarnation
            && self.epoch == other.epoch
            && self.clock == other.clock
    }
}

/// The first stamp rule a candidate violates for a role, fail-closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampFault {
    /// The role does not match the lane it arrived on.
    RoleMismatch,
    /// The clock domain is not legal for the role.
    IllegalClock,
    /// The integrity classification is outside the known vocabulary
    /// (unspecified is a fault, not a default).
    UnknownIntegrity,
}

const fn role_clock_ok(role: u8, clock: u8) -> bool {
    match role {
        // Estimates carry source clocks; truth is simulation-clocked;
        // FC state and navigation guidance are stamped at host receipt.
        ROLE_OPERATIONAL_ESTIMATE => clock == CLOCK_VEHICLE_BOOT || clock == CLOCK_SIMULATION,
        ROLE_SIMULATION_TRUTH => clock == CLOCK_SIMULATION,
        ROLE_FC_STATE | ROLE_NAVIGATION_SOLUTION => clock == CLOCK_HOST_MONOTONIC,
        _ => false,
    }
}

/// Validates the semantic stamp rules for `role`: exact role match, a
/// clock domain that role may legitimately stamp, and a known integrity
/// classification. Wire-shape validation (number bounds, encodings)
/// belongs to the shell that decoded the bytes; everything judgemental
/// lives here so no shell ships weaker provenance checks.
pub fn stamp_fault_for_role(stamp: &RawStamp, role: u8) -> Option<StampFault> {
    if stamp.role != role {
        return Some(StampFault::RoleMismatch);
    }
    if !role_clock_ok(role, stamp.clock) {
        return Some(StampFault::IllegalClock);
    }
    if !KNOWN_INTEGRITY.contains(&stamp.integrity) {
        return Some(StampFault::UnknownIntegrity);
    }
    None
}

pub(crate) fn serial_distance(candidate: u32, current: u32) -> u32 {
    candidate.wrapping_sub(current)
}

/// Wrap-safe serial ordering (RFC 1982 shape): strictly newer when the
/// forward distance is nonzero and below half the range.
pub fn serial_is_newer(candidate: u32, current: u32) -> bool {
    let distance = serial_distance(candidate, current);
    distance != 0 && distance < SERIAL_HALF_RANGE
}

/// Absolute skew between two acquisition instants, valid only after a
/// [`RawStamp::same_stream`] check.
pub(crate) fn skew_ns(a: u64, b: u64) -> u64 {
    a.abs_diff(b)
}

#[cfg(test)]
mod tests;
