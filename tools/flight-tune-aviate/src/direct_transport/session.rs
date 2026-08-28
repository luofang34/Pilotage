//! The authenticated, simulator-only authority for one direct transport.
//!
//! A direct transport exists only where a validated tuning session, a
//! verified simulator binding, and a simulator execution target all agree.
//! Its identity names every part that could change what a direct command
//! means, so a record can be read back against the exact thing that sent
//! it.

use flight_tune::{
    ArtifactIdentity, Digest, ExecutionTarget, SimulatorCapability, SimulatorSessionReceipt,
    VehicleBindingReceipt,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::error::DirectTransportError;

/// Schema version of the direct-transport identity document.
pub const DIRECT_TRANSPORT_IDENTITY_SCHEMA_VERSION: u16 = 1;

const IDENTITY_DIGEST_DOMAIN: &[u8] = b"pilotage-aviate-direct-transport-identity-v1\0";

/// The immutable identity of one simulator-only direct transport.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectTransportIdentity {
    /// Identity document schema version.
    pub schema_version: u16,
    /// The validated tuning session.
    pub session_digest: Digest,
    /// The running simulator.
    pub simulator_digest: Digest,
    /// The loaded airframe.
    pub airframe_digest: Digest,
    /// The bound vehicle adapter.
    pub vehicle_digest: Digest,
    /// The final engine and vehicle action-port runtime.
    pub scenario_runtime_digest: Digest,
    /// The flight-controller command endpoint.
    pub command_endpoint: String,
    /// The direct-transport implementation.
    pub transport: ArtifactIdentity,
}

impl DirectTransportIdentity {
    /// The digest of this identity document.
    ///
    /// # Errors
    ///
    /// Returns [`DirectTransportError`] when the document cannot encode.
    pub fn digest(&self) -> Result<Digest, DirectTransportError> {
        let bytes = serde_json::to_vec(self).map_err(|source| DirectTransportError::Digest {
            artifact: "identity",
            source,
        })?;
        let mut hasher = Sha256::new();
        hasher.update(IDENTITY_DIGEST_DOMAIN);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        Ok(Digest::from_bytes(hasher.finalize().into()))
    }
}

/// The authenticated authority that one direct transport is built from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectTransportSession {
    identity: DirectTransportIdentity,
    identity_digest: Digest,
}

impl DirectTransportSession {
    /// Authorizes one simulator-only direct transport.
    ///
    /// The simulator receipt and the vehicle binding must both name the
    /// tuning session that the capability validated, and the execution
    /// target must be a simulator.
    ///
    /// # Errors
    ///
    /// Returns [`DirectTransportError`] when the target is a real vehicle,
    /// when a binding does not accept the session, or when a bound identity
    /// is missing.
    pub fn authorize(
        capability: &SimulatorCapability,
        simulator: &SimulatorSessionReceipt,
        vehicle: &VehicleBindingReceipt,
        target: ExecutionTarget,
        command_endpoint: &str,
        transport: &ArtifactIdentity,
    ) -> Result<Self, DirectTransportError> {
        if target != ExecutionTarget::Simulator {
            return Err(DirectTransportError::HardwareTarget);
        }
        let session_digest = capability.session_digest();
        if simulator.session_digest != session_digest {
            return Err(DirectTransportError::UnverifiedBinding {
                binding: "simulator",
            });
        }
        if vehicle.session_digest != session_digest {
            return Err(DirectTransportError::UnverifiedBinding { binding: "vehicle" });
        }
        // `ArtifactIdentity::new` is the harness's public validation path:
        // it refuses an empty name and a zero digest.
        ArtifactIdentity::new(transport.id.clone(), transport.digest)
            .map_err(|source| DirectTransportError::InvalidIdentity { source })?;
        let identity = DirectTransportIdentity {
            schema_version: DIRECT_TRANSPORT_IDENTITY_SCHEMA_VERSION,
            session_digest,
            simulator_digest: simulator.simulator_digest,
            airframe_digest: simulator.airframe_digest,
            vehicle_digest: vehicle.vehicle_digest,
            scenario_runtime_digest: vehicle.scenario_runtime_digest,
            command_endpoint: command_endpoint.to_owned(),
            transport: transport.clone(),
        };
        identity.require_complete()?;
        let identity_digest = identity.digest()?;
        Ok(Self {
            identity,
            identity_digest,
        })
    }

    /// The bound transport identity.
    #[must_use]
    pub const fn identity(&self) -> &DirectTransportIdentity {
        &self.identity
    }

    /// The digest of the bound transport identity.
    #[must_use]
    pub const fn identity_digest(&self) -> Digest {
        self.identity_digest
    }

    /// Rejects a session or runtime identity that changed after authorization.
    ///
    /// # Errors
    ///
    /// Returns [`DirectTransportError`] when a binding no longer matches.
    pub fn require_unchanged(
        &self,
        capability: &SimulatorCapability,
        vehicle: &VehicleBindingReceipt,
    ) -> Result<(), DirectTransportError> {
        if capability.session_digest() != self.identity.session_digest
            || vehicle.session_digest != self.identity.session_digest
        {
            return Err(DirectTransportError::ChangedBinding { binding: "session" });
        }
        if vehicle.scenario_runtime_digest != self.identity.scenario_runtime_digest {
            return Err(DirectTransportError::ChangedBinding { binding: "runtime" });
        }
        if vehicle.vehicle_digest != self.identity.vehicle_digest {
            return Err(DirectTransportError::ChangedBinding { binding: "vehicle" });
        }
        Ok(())
    }

    /// Rejects a command endpoint that is not the bound one.
    ///
    /// # Errors
    ///
    /// Returns [`DirectTransportError`] when the endpoint changed.
    pub fn require_endpoint(&self, endpoint: &str) -> Result<(), DirectTransportError> {
        if endpoint == self.identity.command_endpoint {
            return Ok(());
        }
        Err(DirectTransportError::ChangedBinding {
            binding: "command endpoint",
        })
    }
}

impl DirectTransportIdentity {
    fn require_complete(&self) -> Result<(), DirectTransportError> {
        for (name, digest) in [
            ("session", self.session_digest),
            ("simulator", self.simulator_digest),
            ("airframe", self.airframe_digest),
            ("vehicle", self.vehicle_digest),
            ("scenario runtime", self.scenario_runtime_digest),
        ] {
            if digest.is_zero() {
                return Err(DirectTransportError::IncompleteIdentity {
                    detail: format!("the {name} digest is zero"),
                });
            }
        }
        if self.command_endpoint.trim().is_empty() || self.command_endpoint.len() > 256 {
            return Err(DirectTransportError::IncompleteIdentity {
                detail: "the command endpoint is not a short, non-empty name".to_owned(),
            });
        }
        Ok(())
    }
}
