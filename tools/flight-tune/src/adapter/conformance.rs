//! The simulator-neutral open-rollback conformance suite.
//!
//! Every backend and every vehicle factory answers for the resources it
//! creates during one open attempt. The rules are the same whichever
//! simulator is behind them, so the checks live here once and each
//! implementation runs them against its own adapters.

use std::fmt;

use super::{
    CampaignBackend, SimulatorSessionAcquisition, VehicleBindingAcquisition, VehicleBindingRollback,
};

/// One conformance rule that an adapter did not keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceFailure {
    rule: &'static str,
    detail: String,
}

impl ConformanceFailure {
    /// Returns the stable name of the rule the adapter did not keep.
    #[must_use]
    pub const fn rule(&self) -> &'static str {
        self.rule
    }

    /// Returns what the adapter did instead.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }

    fn new(rule: &'static str, detail: impl Into<String>) -> Self {
        Self {
            rule,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for ConformanceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.rule, self.detail)
    }
}

impl std::error::Error for ConformanceFailure {}

/// Checks one simulator-session rollback against the open contract.
///
/// `owned` names a session this backend answers for. `foreign` names one
/// it does not. The backend must hold no session for `owned` when the
/// checks finish.
///
/// # Errors
///
/// Returns [`ConformanceFailure`] when the rollback is not idempotent, or
/// when it accepts an acquisition it does not own.
pub fn check_simulator_session_rollback<B: CampaignBackend>(
    backend: &mut B,
    owned: &SimulatorSessionAcquisition,
    foreign: &SimulatorSessionAcquisition,
) -> Result<(), ConformanceFailure> {
    if owned == foreign {
        return Err(ConformanceFailure::new(
            "session rollback fixture",
            "the owned and foreign acquisitions are the same",
        ));
    }
    backend.close_session_blocking(owned).map_err(|source| {
        ConformanceFailure::new(
            "session rollback proves absence",
            format!("the first close refused an owned acquisition: {source}"),
        )
    })?;
    backend.close_session_blocking(owned).map_err(|source| {
        ConformanceFailure::new(
            "session rollback is idempotent",
            format!("a repeated close changed the result: {source}"),
        )
    })?;
    if backend.close_session_blocking(foreign).is_ok() {
        return Err(ConformanceFailure::new(
            "session rollback refuses a foreign acquisition",
            "the close accepted an acquisition the backend does not own",
        ));
    }
    Ok(())
}

/// Checks one vehicle-binding rollback against the open contract.
///
/// `owned` names a binding this handle answers for. `foreign` names one it
/// does not. The handle must hold no binding for `owned` when the checks
/// finish.
///
/// # Errors
///
/// Returns [`ConformanceFailure`] when the rollback is not idempotent, or
/// when it accepts an acquisition it does not own.
pub fn check_vehicle_binding_rollback<R: VehicleBindingRollback>(
    rollback: &mut R,
    owned: &VehicleBindingAcquisition,
    foreign: &VehicleBindingAcquisition,
) -> Result<(), ConformanceFailure> {
    if owned == foreign {
        return Err(ConformanceFailure::new(
            "vehicle rollback fixture",
            "the owned and foreign acquisitions are the same",
        ));
    }
    rollback.release_binding_blocking(owned).map_err(|source| {
        ConformanceFailure::new(
            "vehicle rollback proves absence",
            format!("the first release refused an owned acquisition: {source}"),
        )
    })?;
    rollback.release_binding_blocking(owned).map_err(|source| {
        ConformanceFailure::new(
            "vehicle rollback is idempotent",
            format!("a repeated release changed the result: {source}"),
        )
    })?;
    if rollback.release_binding_blocking(foreign).is_ok() {
        return Err(ConformanceFailure::new(
            "vehicle rollback refuses a foreign acquisition",
            "the release accepted an acquisition the handle does not own",
        ));
    }
    Ok(())
}

/// Checks both halves of one open rollback in reverse acquisition order.
///
/// # Errors
///
/// Returns [`ConformanceFailure`] when either half breaks the contract.
pub fn check_open_rollback<B: CampaignBackend, R: VehicleBindingRollback>(
    backend: &mut B,
    rollback: &mut R,
    owned_session: &SimulatorSessionAcquisition,
    foreign_session: &SimulatorSessionAcquisition,
    owned_vehicle: &VehicleBindingAcquisition,
    foreign_vehicle: &VehicleBindingAcquisition,
) -> Result<(), ConformanceFailure> {
    check_vehicle_binding_rollback(rollback, owned_vehicle, foreign_vehicle)?;
    check_simulator_session_rollback(backend, owned_session, foreign_session)
}
