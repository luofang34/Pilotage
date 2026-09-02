use pilotage_trial::Digest;

use super::AdapterError;

/// The exact identity of one simulator session an open attempt can hold.
///
/// Cleanup names its target by the identities that authorized the
/// acquisition. A process name or a socket name is a guess about which
/// resource belongs to which attempt; these digests are the attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimulatorSessionAcquisition {
    session_digest: Digest,
    simulator_digest: Digest,
    airframe_digest: Digest,
}

impl SimulatorSessionAcquisition {
    /// Names one simulator session by tuning session, simulator, and airframe.
    #[must_use]
    pub const fn new(
        session_digest: Digest,
        simulator_digest: Digest,
        airframe_digest: Digest,
    ) -> Self {
        Self {
            session_digest,
            simulator_digest,
            airframe_digest,
        }
    }

    /// Returns the tuning session this acquisition belongs to.
    #[must_use]
    pub const fn session_digest(&self) -> Digest {
        self.session_digest
    }

    /// Returns the simulator implementation this acquisition names.
    #[must_use]
    pub const fn simulator_digest(&self) -> Digest {
        self.simulator_digest
    }

    /// Returns the airframe this acquisition names.
    #[must_use]
    pub const fn airframe_digest(&self) -> Digest {
        self.airframe_digest
    }
}

/// The exact identity of one vehicle binding a factory can have created.
///
/// The identity exists before the bind starts, so a bind that fails part
/// way through still has an exact cleanup target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VehicleBindingAcquisition {
    session_digest: Digest,
    vehicle_digest: Digest,
    scenario_runtime_digest: Digest,
}

impl VehicleBindingAcquisition {
    /// Names one vehicle binding by tuning session, vehicle, and runtime.
    #[must_use]
    pub const fn new(
        session_digest: Digest,
        vehicle_digest: Digest,
        scenario_runtime_digest: Digest,
    ) -> Self {
        Self {
            session_digest,
            vehicle_digest,
            scenario_runtime_digest,
        }
    }

    /// Returns the tuning session this acquisition belongs to.
    #[must_use]
    pub const fn session_digest(&self) -> Digest {
        self.session_digest
    }

    /// Returns the vehicle implementation this acquisition names.
    #[must_use]
    pub const fn vehicle_digest(&self) -> Digest {
        self.vehicle_digest
    }

    /// Returns the engine and action-port identity this acquisition names.
    #[must_use]
    pub const fn scenario_runtime_digest(&self) -> Digest {
        self.scenario_runtime_digest
    }
}

/// Releases or contains one exact vehicle binding.
///
/// A factory is consumed by its own bind operation, so the owner of a
/// partial bind has to exist before the bind starts. This handle is that
/// owner.
pub trait VehicleBindingRollback {
    /// Releases or contains the vehicle binding this acquisition names.
    ///
    /// The operation is idempotent: a repeat after an uncertain
    /// acknowledgement keeps the same state and returns the same success.
    /// It returns success only when it proves that no binding for this
    /// acquisition remains. It must not release a resource that another
    /// acquisition owns, and it must not stop an operator-owned runtime.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when the acquisition names a foreign
    /// binding, or when the operation cannot prove that the binding is
    /// absent.
    fn release_binding_blocking(
        &mut self,
        acquisition: &VehicleBindingAcquisition,
    ) -> Result<(), AdapterError>;
}
