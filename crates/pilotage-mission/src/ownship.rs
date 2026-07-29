//! Ownship samples as the host converts them, and the source-role
//! honesty gate the engine holds them to.

use navigate_contract::MonotonicNanos;

/// The LINK-04 source-role vocabulary as the mission engine consumes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TruthRole {
    /// Ground truth exported by a simulator.
    SimulationTruth,
    /// State exported by the flight controller's own estimator.
    FcState,
    /// The host's operational estimate.
    OperationalEstimate,
}

/// One ownship kinematic sample, converted by the host task from
/// vehicle telemetry.
///
/// **Only [`TruthRole::SimulationTruth`] samples become observations.**
/// The engine synthesizes a GNSS-class position fix from truth — a
/// simulation-only stand-in for a real receiver. FC-state and
/// operational-estimate samples are estimator-derived: feeding them back
/// as aids would double-count information the estimator already holds
/// (the ADR-0024 correlation rule), so the engine refuses them with a
/// counted rejection and never launders them into a fix. A physical
/// vehicle needs a genuine independent position source before this
/// engine can guide it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnshipSample {
    /// Position as NED meters from the mission anchor.
    pub ned: [f64; 3],
    /// Velocity in NED, m/s. The synthesized observation is
    /// position-only; velocity aiding is a designed extension.
    pub ned_velocity: [f64; 3],
    /// Heading, radians, zero north, positive toward east. Absent when
    /// the source carried no attitude group: the engine can still feed
    /// fusion from position, but it cannot rotate NED into the body
    /// frame honestly, so a tick without a known heading emits no
    /// intent (zero would silently rotate commands to due north).
    pub yaw_rad: Option<f64>,
    /// Which estimator family produced this sample.
    pub role: TruthRole,
    /// Monotonic acquisition time on the engine's clock domain.
    pub acquired_at: MonotonicNanos,
    /// Per-source sample sequence, wrap-aware.
    pub sequence: u32,
}
