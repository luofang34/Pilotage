//! The session profile vocabulary the adapter is constructed for.

/// Which session profile the adapter runs (LINK-04). A profile binds
/// source ROLES — the MAVLink link carries the FC operational estimate,
/// the shm block carries simulation truth — and transports are never
/// alternatives for one another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AviateProfile {
    /// Physical vehicle: FC estimate + FC state. A truth source must not
    /// exist and is never synthesized.
    Physical,
    /// Simulation: FC estimate + FC state, plus the simulation-truth
    /// oracle while the co-located shm block is attachable.
    #[default]
    Simulation,
    /// Oracle-only diagnostics: the truth stream alone. No uplink is
    /// bound and no motion-control scope is advertised — operational
    /// control is structurally absent, not merely rejected.
    OracleOnly,
}
