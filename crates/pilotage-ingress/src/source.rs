//! Source identity vocabulary shared by every ingress path.

/// The monotonic clock in which a measurement's acquisition timestamp is
/// expressed. Timestamps from different domains are never subtracted without
/// an explicit correlation supplied by the adapter boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasurementClock {
    /// Monotonic time since the producing vehicle computer booted.
    VehicleBoot,
    /// Monotonic simulation time supplied by the simulator.
    Simulation,
    /// Monotonic time on the ground host that received the observation,
    /// for reports whose wire carries no source timestamp (an FC
    /// heartbeat). Receive time is not acquisition time; consumers may
    /// only reason about staleness in this domain, never correlate it
    /// with vehicle or simulation clocks without an explicit mapping.
    HostMonotonic,
}

/// Opaque attachment or boot identity for a producing source.
///
/// Unlike an epoch, this value is compared only for equality. A new
/// incarnation cannot be ordered relative to an earlier one; the receiver must
/// authorize that transition at a lifecycle boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceIncarnation([u8; 16]);

impl SourceIncarnation {
    /// Constructs an incarnation from its complete 128-bit representation.
    #[must_use]
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the complete opaque representation.
    #[must_use]
    pub const fn into_bytes(self) -> [u8; 16] {
        self.0
    }
}

/// Explicit role of the source behind a measurement.
///
/// Role is carried in provenance — never encoded into id ranges — so a
/// configured source id can collide across roles without ambiguity, and
/// consumers gate on the role itself (panels and control accept only
/// [`SourceRole::OperationalEstimate`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRole {
    /// Estimator output: the only role eligible for primary panels and
    /// operational command construction.
    OperationalEstimate,
    /// Simulator ground truth: logging, assertions, and comparison in
    /// simulation profiles only.
    SimulationTruth,
    /// Vehicle state (arm/mode/failsafe) reports.
    FcState,
    /// Video capture identity for camera frames.
    VideoCapture,
    /// Payload-device orientation or state relayed over the vehicle link:
    /// never a vehicle estimate, never eligible for control validation,
    /// carrying the device's own boot clock.
    PayloadDevice,
}

/// Integrity classification of the path that delivered an observation.
///
/// Every role carries it so authenticated, checksummed, and unprotected
/// inputs stay distinguishable end to end. The distinction that matters
/// end-to-end is authenticated source data versus merely checksummed or
/// unprotected observations; a consumer making a safety claim must require
/// the level the claim needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceIntegrity {
    /// Cryptographically authenticated end-to-end source data.
    Authenticated,
    /// Checksummed (CRC-style) but unauthenticated transport.
    ChecksummedOnly,
    /// No integrity protection beyond the transport's own boundaries
    /// (a local shared-memory mapping relies on host process isolation).
    Unprotected,
}
