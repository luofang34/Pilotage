//! The `VehicleAdapter` boundary trait (ADR-0008).

use pilotage_protocol::{ScopeId, ScopedControlFrame, VehicleId};

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

    /// Sets or clears the link-loss policy a vehicle should follow, PER SCOPE,
    /// when a scope's control link is judged lost.
    ///
    /// The latch is scope-specific: engaging `vehicle.gimbal` must not suppress
    /// or neutralize `vehicle.motion`, and vice versa. `Some(policy)` engages
    /// that policy for `scope`; while a scope's policy is engaged the adapter
    /// must suppress ordinary control frames FOR THAT SCOPE (rejecting them with
    /// [`RejectReason::LinkLossEngaged`]) so a newly granted holder cannot drive
    /// that scope out of its policy state before the host clears it, and must
    /// drive only that scope's actuation to its safe state. `None` signals link
    /// recovery for `scope`: it clears that scope's engaged policy and returns
    /// the scope to normal control, the only API-level path back once a scope's
    /// policy has been engaged (ADR-0008).
    ///
    /// # Latch postcondition (asymmetric, always fail-closed)
    ///
    /// - **Engage (`Some`)**: the latch is recorded and the scope stays
    ///   suppressed **even if the actuation is refused** (`Err`) — a fenced
    ///   scope must never be left drivable, so a failed engage still rejects
    ///   that scope's control.
    /// - **Clear (`None`)**: the latch is removed and the scope returns to
    ///   normal control **only on `Ok`**. On `Err` the latch **stays engaged**
    ///   (the scope keeps rejecting control) for the caller to retry. A scope is
    ///   un-suppressed only by a clear the adapter accepted, never speculatively.
    ///
    /// Actuation is best-effort at the link boundary: `Ok` means the safe-state
    /// command reached the vehicle link, not that the vehicle confirmed it. An
    /// adapter that CAN confirm SHOULD keep the scope engaged/faulted until the
    /// vehicle confirms the safe state.
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
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError>;

    /// Advances the adapter by up to `budget` ticks; the primary drive
    /// mechanism for stepped, deterministic, and accelerated execution
    /// (ADR-0013).
    fn step(&mut self, budget: StepBudget) -> StepOutcome;
}
