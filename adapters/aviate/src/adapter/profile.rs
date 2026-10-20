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
    /// oracle that [`AviateTruth`] names, while it is attachable.
    #[default]
    Simulation,
    /// Oracle-only diagnostics: the truth stream alone. No uplink is
    /// bound and no motion-control scope is advertised — operational
    /// control is structurally absent, not merely rejected.
    OracleOnly,
}

/// Where a simulation session's truth comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AviateTruth {
    /// The co-located simulator's shm block, which the adapter attaches
    /// as its truth oracle (Gazebo).
    #[default]
    SharedMemory,
    /// A truth lane the host binds itself (a JSBSim sidecar): the adapter
    /// attaches no oracle, so another simulator's shm block on the same
    /// machine never stands in for this vehicle.
    HostLane,
}
