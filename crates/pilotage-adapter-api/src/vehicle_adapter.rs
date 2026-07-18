//! The `VehicleAdapter` boundary trait (ADR-0008).

use pilotage_protocol::{ScopedControlFrame, VehicleId};

use crate::capability::AdapterCapabilities;
use crate::control::{ApplyOutcome, LinkLossEnactError, LinkLossPolicy};
use crate::step::{StepBudget, StepOutcome};
use crate::telemetry::{TelemetryBatch, VideoSource};

/// The engine-independent boundary between the session host and a vehicle
/// simulation or gateway (ADR-0008).
///
/// Sans-IO per ADR-0002: implementations may wrap an engine, a headless
/// simulation, or a real-vehicle gateway, but this trait itself performs no
/// I/O and reads no system clock. Time enters only as the simulation tick an
/// adapter reports back in its outcomes.
pub trait VehicleAdapter {
    /// Reports the vehicles, scopes, and execution characteristics this
    /// adapter supports.
    fn capabilities(&self) -> AdapterCapabilities;

    /// Applies a single scoped control frame, returning how it was disposed
    /// of and the simulation tick the outcome corresponds to.
    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome;

    /// Samples current telemetry for all vehicles this adapter exposes.
    fn sample_telemetry(&mut self) -> TelemetryBatch;

    /// Lists video or camera sources this adapter exposes; empty for
    /// adapters that are not `render_capable`.
    fn video_sources(&self) -> Vec<VideoSource>;

    /// Sets or clears the link-loss policy a vehicle should follow when its
    /// control link is judged lost.
    ///
    /// `Some(policy)` engages that policy; `Some` replaces whatever policy
    /// was previously engaged. While a policy is engaged the adapter must
    /// suppress ordinary control frames (rejecting them with
    /// [`RejectReason::LinkLossEngaged`]) so a newly granted holder cannot
    /// drive the vehicle out of its policy state before the host clears it.
    /// `None` signals link recovery: it clears any engaged policy and
    /// returns the vehicle to normal control, which is the only API-level
    /// path back to normal control once a policy has been engaged
    /// (ADR-0008).
    ///
    /// # Errors
    ///
    /// Returns the typed enactment failure when the policy change could not
    /// be driven to the vehicle. The caller must treat any error as a
    /// counted fail-closed fault — authority has already been fenced, so an
    /// unenacted policy leaves the vehicle executing its last command with
    /// nobody in control.
    ///
    /// [`RejectReason::LinkLossEngaged`]: crate::RejectReason::LinkLossEngaged
    fn set_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError>;

    /// Advances the adapter by up to `budget` ticks; the primary drive
    /// mechanism for stepped, deterministic, and accelerated execution
    /// (ADR-0013).
    fn step(&mut self, budget: StepBudget) -> StepOutcome;
}
