//! The ADR-0011 message classes of the engine's fan-out.

/// The ADR-0011 message classes the engine actor fans messages out as, for
/// per-class drop accounting (ADR-0009: "drops are counted, never silent";
/// ADR-0011: "drops are counted per class").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MessageClass {
    /// A unicast reply to one client (bootstrap/session state or `Pong`).
    Unicast,
    /// A reliable, ordered authority/mode-change broadcast.
    AuthorityBroadcast,
    /// Best-effort telemetry, fanned out at tick cadence.
    Telemetry,
}
