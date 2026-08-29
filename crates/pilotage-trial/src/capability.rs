//! Backend capability declarations.

use serde::{Deserialize, Serialize};

use crate::{
    ArtifactIdentity, BACKEND_CAPABILITIES_SCHEMA_VERSION, CodecError, Digest, MAX_CAPABILITIES,
    MAX_MANIFEST_BYTES, ValidationError, canonical,
    validation::{count, schema, unique},
};

#[cfg(test)]
mod tests;

/// A capability that a trial backend can supply.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendCapability {
    /// Reset the simulated vehicle and world.
    Reset,
    /// Report the simulator lifecycle state.
    LifecycleState,
    /// Report and control simulator time.
    SimulatorTime,
    /// Apply an environmental condition set.
    ConditionControl,
    /// Report kinematic truth data.
    KinematicTruth,
    /// Apply a deterministic random seed.
    DeterministicSeed,
    /// Arm and disarm the vehicle.
    ArmDisarm,
    /// Report ground contact and crash states.
    ContactState,
    /// Apply a controlled wind field.
    WindControl,
    /// Apply controlled turbulence.
    TurbulenceControl,
    /// Command the operator velocity control family.
    OperatorVelocityControl,
    /// Command the direct attitude and thrust control family.
    DirectAttitudeThrustControl,
    /// Apply deterministic bounded sensor perturbations.
    SensorPerturbation,
    /// Scale eligible actuator commands.
    ActuatorAuthority,
    /// Apply a deterministic command zero-order hold.
    CommandHold,
    /// Scale the controller hover-force initialization.
    HoverTrimUncertainty,
}

impl BackendCapability {
    /// Gets the stable capability name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reset => "reset",
            Self::LifecycleState => "lifecycle_state",
            Self::SimulatorTime => "simulator_time",
            Self::ConditionControl => "condition_control",
            Self::KinematicTruth => "kinematic_truth",
            Self::DeterministicSeed => "deterministic_seed",
            Self::ArmDisarm => "arm_disarm",
            Self::ContactState => "contact_state",
            Self::WindControl => "wind_control",
            Self::TurbulenceControl => "turbulence_control",
            Self::OperatorVelocityControl => "operator_velocity_control",
            Self::DirectAttitudeThrustControl => "direct_attitude_thrust_control",
            Self::SensorPerturbation => "sensor_perturbation",
            Self::ActuatorAuthority => "actuator_authority",
            Self::CommandHold => "command_hold",
            Self::HoverTrimUncertainty => "hover_trim_uncertainty",
        }
    }
}

/// The online hover-estimator state for one backend.
///
/// A hover-force uncertainty request needs a value that the controller keeps.
/// An online estimator writes over that value, so the request is refused.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoverEstimatorMode {
    /// The estimator can update the hover-force value.
    #[default]
    Online,
    /// The estimator is disabled.
    Disabled,
    /// The estimator keeps one fixed value.
    Frozen,
}

impl HoverEstimatorMode {
    /// Reports whether hover-force uncertainty can use this mode.
    #[must_use]
    pub const fn is_inactive(self) -> bool {
        matches!(self, Self::Disabled | Self::Frozen)
    }

    /// Gets the stable estimator-mode name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Disabled => "disabled",
            Self::Frozen => "frozen",
        }
    }
}

/// A versioned declaration of backend capabilities.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendCapabilities {
    /// The backend capabilities schema version.
    pub schema_version: u16,
    /// The backend artifact identity.
    pub backend: ArtifactIdentity,
    /// The capabilities that the backend supplies.
    pub capabilities: Vec<BackendCapability>,
    /// The online hover-estimator state.
    pub hover_estimator_mode: HoverEstimatorMode,
}

impl BackendCapabilities {
    /// Decodes and validates a backend capabilities JSON document.
    pub fn from_json(bytes: &[u8]) -> Result<Self, CodecError> {
        let value: Self = canonical::decode("backend capabilities", bytes, MAX_MANIFEST_BYTES)?;
        value.validate()?;
        Ok(value)
    }

    /// Validates the backend capability declaration.
    pub fn validate(&self) -> Result<(), ValidationError> {
        schema(
            "backend capabilities",
            self.schema_version,
            BACKEND_CAPABILITIES_SCHEMA_VERSION,
        )?;
        self.backend.validate("backend_capabilities.backend")?;
        count(
            "backend_capabilities.capabilities",
            self.capabilities.len(),
            MAX_CAPABILITIES,
        )?;
        unique("backend_capabilities.capabilities", &self.capabilities)
    }

    /// Reports if the backend supplies a capability.
    #[must_use]
    pub fn supports(&self, capability: BackendCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Encodes canonical compact JSON after validation.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, CodecError> {
        self.validate()?;
        canonical::encode("backend capabilities", self, MAX_MANIFEST_BYTES)
    }

    /// Calculates the digest of canonical compact JSON.
    pub fn canonical_digest(&self) -> Result<Digest, CodecError> {
        self.to_canonical_json()
            .map(|bytes| canonical::digest(&bytes))
    }
}
