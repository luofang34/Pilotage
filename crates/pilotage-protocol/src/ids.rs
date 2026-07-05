//! Identifier newtypes shared across the protocol, authority, and input
//! crates.
//!
//! Numeric identifiers are opaque `u64`/`u32` wrappers so callers cannot
//! accidentally mix a `SessionId` with a `VehicleId` at the type level.
//! `ScopeId` is a string because scopes are host-published vocabulary
//! (ADR-0006), not a fixed enum.

/// Identifies a session between a principal and a session host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(u64);

impl SessionId {
    /// Constructs a session identifier from a raw value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the identifier as a raw `u64`.
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

/// Identifies an authenticated principal (a user or service identity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PrincipalId(u64);

impl PrincipalId {
    /// Constructs a principal identifier from a raw value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the identifier as a raw `u64`.
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

/// Identifies a vehicle under remote control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VehicleId(u64);

impl VehicleId {
    /// Constructs a vehicle identifier from a raw value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the identifier as a raw `u64`.
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

/// Identifies an independently assignable control scope (e.g.
/// `"vehicle.motion"`, `"vehicle.camera"`).
///
/// Scopes are published by host capability discovery rather than hard-coded
/// globally, per ADR-0006, so this is a string newtype rather than an enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeId(String);

impl ScopeId {
    /// Constructs a scope identifier from any string-like value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the scope identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A fencing generation for a control-scope lease (ADR-0006).
///
/// Advances on every handover, revocation, override, or reassignment. Uses
/// `wrapping_add` so a long-lived session cannot panic on overflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Generation(u64);

impl Generation {
    /// Constructs a generation from a raw value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the generation as a raw `u64`.
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    /// Returns the next generation, wrapping on overflow rather than
    /// panicking.
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// A monotonically increasing sequence number for ordering control frames
/// within a scope (ADR-0009, ADR-0011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SequenceNum(u32);

impl SequenceNum {
    /// Constructs a sequence number from a raw value.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the sequence number as a raw `u32`.
    #[must_use]
    pub const fn as_u32(&self) -> u32 {
        self.0
    }

    /// Returns the next sequence number, wrapping on overflow rather than
    /// panicking.
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Generation, ScopeId, SequenceNum};

    #[test]
    fn generation_next_wraps_on_overflow() {
        let generation = Generation::new(u64::MAX);
        assert_eq!(generation.next().as_u64(), 0);
    }

    #[test]
    fn generation_next_advances() {
        let generation = Generation::new(3);
        assert_eq!(generation.next().as_u64(), 4);
    }

    #[test]
    fn sequence_num_next_wraps_on_overflow() {
        let seq = SequenceNum::new(u32::MAX);
        assert_eq!(seq.next().as_u32(), 0);
    }

    #[test]
    fn scope_id_holds_published_string() {
        let scope = ScopeId::new("vehicle.motion");
        assert_eq!(scope.as_str(), "vehicle.motion");
    }
}
