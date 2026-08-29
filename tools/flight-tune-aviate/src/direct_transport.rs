//! The simulator-only exact direct transport for an Aviate vehicle.
//!
//! The normal Pilotage direct path rate-limits and jerk-limits every
//! request, so a requested step arrives at the flight controller as a
//! ramp. Evidence collected through it measures Pilotage shaping and not
//! the direct controller. This transport delivers the step itself.
//!
//! It is not reachable from the normal application control interface. It
//! exists only where a validated tuning session, a verified simulator
//! binding, and a simulator execution target all agree, and it owns no
//! socket: it borrows the command sender that already carries the
//! vehicle's normal command stream.
//!
//! The operator velocity family keeps the normal control-feel path with
//! its response curve, neutral hysteresis, apply dynamics, release
//! dynamics, and hold transition. This transport refuses that family.
//!
//! SIM / NOT FOR FLIGHT.

mod baseline;
mod command;
mod error;
mod port;
mod readback;
mod record;
mod revoke;
mod session;
mod step;
#[cfg(test)]
mod tests;

use flight_tune::{
    ArtifactIdentity, ControlFamily, Digest, ExecutionTarget, SimulatorCapability,
    SimulatorSessionReceipt, TuneError, VehicleBindingReceipt,
};
use sha2::{Digest as _, Sha256};

pub use baseline::{DirectBaseline, DirectBaselineRequest};
pub use error::DirectTransportError;
pub use port::{
    DirectCommandSender, DirectSenderError, DirectSenderIdentity, DirectSetpoint,
    EffectiveSetpointReport, TransmittedDirectCommand,
};
pub use readback::{CausalReadbackBound, ReadbackSelection};
pub use record::{DIRECT_COMMAND_RECORD_SCHEMA_VERSION, DirectCommandRecord, DirectCommandTimes};
pub use revoke::DirectRevokeReceipt;
pub use session::{
    DIRECT_TRANSPORT_IDENTITY_SCHEMA_VERSION, DirectTransportIdentity, DirectTransportSession,
};
pub use step::{DirectCommandPurpose, DirectStepRequest, PreparedDirectCommand};

const TRANSPORT_IDENTITY_DOMAIN: &[u8] = b"pilotage-aviate-direct-transport-v1\0";

/// The exact source files of the direct-transport implementation.
const TRANSPORT_SOURCES: [(&str, &[u8]); 10] = [
    ("direct_transport.rs", include_bytes!("direct_transport.rs")),
    (
        "direct_transport/baseline.rs",
        include_bytes!("direct_transport/baseline.rs"),
    ),
    (
        "direct_transport/command.rs",
        include_bytes!("direct_transport/command.rs"),
    ),
    (
        "direct_transport/error.rs",
        include_bytes!("direct_transport/error.rs"),
    ),
    (
        "direct_transport/port.rs",
        include_bytes!("direct_transport/port.rs"),
    ),
    (
        "direct_transport/readback.rs",
        include_bytes!("direct_transport/readback.rs"),
    ),
    (
        "direct_transport/record.rs",
        include_bytes!("direct_transport/record.rs"),
    ),
    (
        "direct_transport/revoke.rs",
        include_bytes!("direct_transport/revoke.rs"),
    ),
    (
        "direct_transport/session.rs",
        include_bytes!("direct_transport/session.rs"),
    ),
    (
        "direct_transport/step.rs",
        include_bytes!("direct_transport/step.rs"),
    ),
];

/// The exact production sources that the transport identity binds.
///
/// A new production source under `direct_transport` has to enter this
/// list, or the exact direct step it shapes would not reach the runtime
/// production-input identity that qualifies it.
#[must_use]
pub fn direct_transport_sources() -> Vec<&'static str> {
    TRANSPORT_SOURCES.iter().map(|(name, _)| *name).collect()
}

/// The content identity of this direct-transport implementation.
///
/// # Errors
///
/// Returns [`TuneError`] when the identity cannot be named.
pub fn direct_transport_identity() -> Result<ArtifactIdentity, TuneError> {
    let mut hasher = Sha256::new();
    hasher.update(TRANSPORT_IDENTITY_DOMAIN);
    for (name, source) in TRANSPORT_SOURCES {
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update((source.len() as u64).to_le_bytes());
        hasher.update(source);
    }
    ArtifactIdentity::new(
        "pilotage-aviate-direct-transport-v1",
        Digest::from_bytes(hasher.finalize().into()),
    )
}

/// What one simulator-only direct transport is authorized from.
#[derive(Clone, Copy, Debug)]
pub struct DirectTransportRequest<'binding> {
    /// The capability for the validated tuning session.
    pub capability: &'binding SimulatorCapability,
    /// The verified simulator handshake receipt.
    pub simulator: &'binding SimulatorSessionReceipt,
    /// The verified vehicle binding receipt.
    pub vehicle: &'binding VehicleBindingReceipt,
    /// The mission execution target.
    pub target: ExecutionTarget,
    /// The direct-transport implementation identity.
    pub transport: &'binding ArtifactIdentity,
    /// The causal rule for accepting a raw readback sample.
    pub readback: CausalReadbackBound,
    /// The declared numeric tolerance for a target comparison.
    pub tolerance: f64,
}

/// The outcome of enacting one prepared direct command.
#[derive(Clone, Debug, PartialEq)]
pub enum DirectEnactment {
    /// The command reached the flight controller with a complete record.
    Enacted(Box<DirectCommandRecord>),
    /// The raw source has not reached the command time. Nothing was sent.
    Pending,
    /// The raw source carries no exact sample for the command time.
    ///
    /// Nothing was sent, and no step or release marker was recorded. The
    /// direct phase never falls back to a delayed estimate.
    NoExactSource,
}

/// One simulator-only exact direct transport.
#[derive(Debug)]
pub struct DirectTransport {
    session: DirectTransportSession,
    readback: CausalReadbackBound,
    tolerance: f64,
    baseline: Option<DirectBaseline>,
    revoked: bool,
}

impl DirectTransport {
    /// Authorizes one simulator-only direct transport.
    ///
    /// # Errors
    ///
    /// Returns [`DirectTransportError`] when the execution target is a real
    /// vehicle, when a binding does not accept the tuning session, when a
    /// bound identity is missing, or when the tolerance is unusable.
    pub fn authorize<S: DirectCommandSender + ?Sized>(
        request: &DirectTransportRequest<'_>,
        sender: &S,
    ) -> Result<Self, DirectTransportError> {
        if !request.tolerance.is_finite() || request.tolerance < 0.0 {
            return Err(DirectTransportError::InvalidValue {
                field: "numeric tolerance",
            });
        }
        let session = DirectTransportSession::authorize(
            request.capability,
            request.simulator,
            request.vehicle,
            request.target,
            &sender.command_endpoint(),
            request.transport,
        )?;
        Ok(Self {
            session,
            readback: request.readback,
            tolerance: request.tolerance,
            baseline: None,
            revoked: false,
        })
    }

    /// The authenticated authority this transport holds.
    #[must_use]
    pub const fn session(&self) -> &DirectTransportSession {
        &self.session
    }

    /// The frozen direct baseline, once the run has one.
    #[must_use]
    pub const fn baseline(&self) -> Option<&DirectBaseline> {
        self.baseline.as_ref()
    }

    /// The causal rule for accepting a raw readback sample.
    #[must_use]
    pub const fn readback_bound(&self) -> CausalReadbackBound {
        self.readback
    }

    /// The declared numeric tolerance for a target comparison.
    #[must_use]
    pub const fn tolerance(&self) -> f64 {
        self.tolerance
    }

    /// Whether this transport still holds direct authority.
    #[must_use]
    pub const fn is_revoked(&self) -> bool {
        self.revoked
    }

    /// Rejects a session or runtime identity that changed after authorization.
    ///
    /// # Errors
    ///
    /// Returns [`DirectTransportError`] when a binding no longer matches or
    /// the authority is revoked.
    pub fn require_binding(
        &self,
        capability: &SimulatorCapability,
        vehicle: &VehicleBindingReceipt,
    ) -> Result<(), DirectTransportError> {
        self.require_authority()?;
        self.session.require_unchanged(capability, vehicle)
    }

    /// Removes every direct authority this transport holds.
    ///
    /// The operation is idempotent. A second call removes nothing and
    /// returns the same receipt.
    pub fn revoke(&mut self) -> DirectRevokeReceipt {
        let removed_authority = !self.revoked;
        let released_baseline = self.baseline.take().is_some();
        self.revoked = true;
        DirectRevokeReceipt::new(
            self.session.identity_digest(),
            removed_authority,
            released_baseline,
        )
    }

    pub(crate) const fn require_authority(&self) -> Result<(), DirectTransportError> {
        if self.revoked {
            return Err(DirectTransportError::Revoked);
        }
        Ok(())
    }

    pub(crate) fn require_direct_stimulus(
        &self,
        family: ControlFamily,
    ) -> Result<(), DirectTransportError> {
        self.require_authority()?;
        step::require_direct_family(family)
    }
}
