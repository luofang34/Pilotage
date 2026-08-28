//! Mission document, phase, and execution-policy types.

use serde::{Deserialize, Serialize};

use crate::{
    CodecError, Digest, MAX_CAPABILITIES, MAX_CLEANUP_ACTIONS, MAX_PHASE_CONDITIONS, MAX_PHASES,
    MISSION_SCHEMA_VERSION, MissionAction, MissionCondition, MissionIdentity,
    NavigationDataIdentity, TransportLane, ValidationError, canonical, validation,
};

/// A mission execution target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionTarget {
    /// A simulator target.
    Simulator,
    /// A real-vehicle target.
    RealVehicle,
}

/// The execution policy for one mission document.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPolicy {
    /// The target that can execute the document.
    pub target: ExecutionTarget,
    /// The maximum number of retries after the first directive attempt.
    pub retry_limit: u16,
    /// The maximum caller wall-clock wait for one directive receipt.
    pub receipt_timeout_ns: u64,
}

/// A capability required by a mission phase.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum MissionCapability {
    /// Reset a simulated vehicle and world.
    Reset,
    /// Report lifecycle state.
    LifecycleState,
    /// Report and control simulator time.
    SimulatorTime,
    /// Apply an environmental condition set.
    ConditionControl,
    /// Report kinematic truth data.
    KinematicTruth,
    /// Apply a deterministic random seed.
    DeterministicSeed,
    /// Arm and disarm a vehicle.
    ArmDisarm,
    /// Report ground-contact and crash state.
    ContactState,
    /// Apply a controlled wind field.
    WindControl,
    /// Apply controlled turbulence.
    TurbulenceControl,
    /// Control operational flight targets.
    FlightControl,
    /// Resolve and follow an immutable flight plan.
    FlightPlan,
    /// Report navigation state.
    NavigationState,
    /// Apply simulator-only trial control.
    SimulatorControl,
    /// Command the operator velocity control family.
    OperatorVelocityControl,
    /// Command the direct attitude and thrust control family.
    DirectAttitudeThrustControl,
}

/// One bounded phase in a mission document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionPhase {
    /// The stable phase identifier.
    pub id: String,
    /// The capabilities that the phase needs.
    pub required_capabilities: Vec<MissionCapability>,
    /// The conditions that permit phase entry.
    pub entry_conditions: Vec<MissionCondition>,
    /// The one action for the phase.
    pub action: MissionAction,
    /// The actions to attempt during abort cleanup.
    pub cleanup_actions: Vec<MissionAction>,
    /// The conditions that complete the phase.
    pub completion_conditions: Vec<MissionCondition>,
    /// The conditions that abort the mission.
    pub abort_conditions: Vec<MissionCondition>,
    /// The maximum phase duration in simulator nanoseconds.
    pub simulator_time_deadline_ns: u64,
}

/// A canonical mission document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionDocument {
    /// The immutable document identity.
    pub identity: MissionIdentity,
    /// The execution policy.
    pub execution_policy: ExecutionPolicy,
    /// The ordered mission phases.
    pub phases: Vec<MissionPhase>,
}

impl MissionDocument {
    /// Creates and validates a mission document with a calculated content digest.
    ///
    /// # Errors
    ///
    /// Returns an error if the content is invalid or cannot be encoded.
    pub fn new(
        revision_id: String,
        navigation_data_identity: NavigationDataIdentity,
        execution_policy: ExecutionPolicy,
        phases: Vec<MissionPhase>,
    ) -> Result<Self, CodecError> {
        let mut document = Self {
            identity: MissionIdentity {
                revision_id,
                schema_version: MISSION_SCHEMA_VERSION,
                content_digest: Digest::ZERO,
                navigation_data_identity,
            },
            execution_policy,
            phases,
        };
        document.validate_content()?;
        document.identity.content_digest = document.calculate_content_digest()?;
        document.validate()?;
        Ok(document)
    }

    /// Decodes and validates a mission document from JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if decoding, validation, or digest verification fails.
    pub fn from_json(bytes: &[u8]) -> Result<Self, CodecError> {
        let document: Self = canonical::decode(bytes)?;
        document.validate()?;
        document.verify_content_digest()?;
        Ok(document)
    }

    /// Encodes validated canonical compact JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if validation, digest verification, or encoding fails.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, CodecError> {
        self.validate()?;
        self.verify_content_digest()?;
        canonical::encode(self)
    }

    /// Calculates the domain-separated digest of the canonical mission content.
    ///
    /// The digest input excludes `identity.content_digest`.
    ///
    /// # Errors
    ///
    /// Returns an error if the canonical content cannot be encoded.
    pub fn calculate_content_digest(&self) -> Result<Digest, CodecError> {
        canonical::content_digest(self)
    }

    /// Validates the document for its declared execution target.
    ///
    /// # Errors
    ///
    /// Returns an error if a field, phase, capability, or action is invalid.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.validate_for_target(self.execution_policy.target)
    }

    /// Validates the document for a host execution target.
    ///
    /// # Errors
    ///
    /// Returns an error if the host target is not permitted or content is invalid.
    pub fn validate_for_target(&self, target: ExecutionTarget) -> Result<(), ValidationError> {
        self.identity.validate_fields()?;
        self.execution_policy.validate()?;
        self.validate_phases()?;
        self.validate_admission(target)?;
        if self.execution_policy.target != target {
            return Err(ValidationError::ExecutionTargetMismatch {
                document_target: self.execution_policy.target,
                host_target: target,
            });
        }
        Ok(())
    }

    fn validate_content(&self) -> Result<(), ValidationError> {
        self.identity.validate_content_fields()?;
        self.execution_policy.validate()?;
        self.validate_phases()
    }

    fn validate_phases(&self) -> Result<(), ValidationError> {
        validation::nonempty_count("mission.phases", self.phases.len(), MAX_PHASES)?;
        for (index, phase) in self.phases.iter().enumerate() {
            phase.validate(index, &self.identity.navigation_data_identity)?;
            if self.phases[..index]
                .iter()
                .any(|prior| prior.id == phase.id)
            {
                return Err(ValidationError::RepeatedPhaseId {
                    phase_id: phase.id.clone(),
                    index,
                });
            }
        }
        Ok(())
    }

    fn validate_admission(&self, target: ExecutionTarget) -> Result<(), ValidationError> {
        if target != ExecutionTarget::RealVehicle {
            return Ok(());
        }
        for phase in &self.phases {
            for action in std::iter::once(&phase.action).chain(&phase.cleanup_actions) {
                if action.transport_lane() == TransportLane::SimulatorOnly {
                    return Err(ValidationError::SimulatorOnlyAction {
                        phase_id: phase.id.clone(),
                        action: action.name(),
                    });
                }
            }
        }
        Ok(())
    }

    pub(crate) fn verify_content_digest(&self) -> Result<(), CodecError> {
        self.identity.validate_fields()?;
        let calculated = self.calculate_content_digest()?;
        if self.identity.content_digest != calculated {
            return Err(ValidationError::ContentDigestMismatch {
                declared: self.identity.content_digest,
                calculated,
            }
            .into());
        }
        Ok(())
    }
}

impl MissionPhase {
    fn validate(
        &self,
        index: usize,
        navigation_data: &NavigationDataIdentity,
    ) -> Result<(), ValidationError> {
        let field = format!("mission.phases[{index}]");
        validation::text(&format!("{field}.id"), &self.id)?;
        self.validate_deadline()?;
        validation::count_with_limit(
            &format!("{field}.required_capabilities"),
            self.required_capabilities.len(),
            MAX_CAPABILITIES,
        )?;
        self.validate_capability_list()?;
        self.validate_conditions(&field)?;
        self.action.validate(&field)?;
        self.validate_cleanup_actions(&field)?;
        self.validate_plan_identity(&self.action, navigation_data)?;
        for action in &self.cleanup_actions {
            self.validate_plan_identity(action, navigation_data)?;
        }
        self.validate_capability_declarations()
    }

    fn validate_deadline(&self) -> Result<(), ValidationError> {
        if self.simulator_time_deadline_ns == 0 {
            return Err(ValidationError::MissingDeadline {
                phase_id: self.id.clone(),
            });
        }
        Ok(())
    }

    fn validate_capability_list(&self) -> Result<(), ValidationError> {
        if let Some(capability) = validation::duplicate(&self.required_capabilities) {
            return Err(ValidationError::RepeatedCapability {
                phase_id: self.id.clone(),
                capability,
            });
        }
        Ok(())
    }

    fn validate_conditions(&self, field: &str) -> Result<(), ValidationError> {
        validate_condition_list(field, "entry_conditions", &self.entry_conditions)?;
        validate_condition_list(field, "completion_conditions", &self.completion_conditions)?;
        validate_condition_list(field, "abort_conditions", &self.abort_conditions)
    }

    fn validate_cleanup_actions(&self, field: &str) -> Result<(), ValidationError> {
        validation::count_with_limit(
            &format!("{field}.cleanup_actions"),
            self.cleanup_actions.len(),
            MAX_CLEANUP_ACTIONS,
        )?;
        for (index, action) in self.cleanup_actions.iter().enumerate() {
            action.validate(&format!("{field}.cleanup_actions[{index}]"))?;
        }
        Ok(())
    }

    fn validate_plan_identity(
        &self,
        action: &MissionAction,
        navigation_data: &NavigationDataIdentity,
    ) -> Result<(), ValidationError> {
        if let Some(plan) = action.flight_plan()
            && plan.navigation_data_identity != *navigation_data
        {
            return Err(ValidationError::NavigationDataMismatch {
                phase_id: self.id.clone(),
                plan_id: plan.plan_id.clone(),
            });
        }
        Ok(())
    }

    fn validate_capability_declarations(&self) -> Result<(), ValidationError> {
        self.require_declared(MissionCapability::SimulatorTime)?;
        if let Some(capability) = self.action.required_capability() {
            self.require_declared(capability)?;
        }
        for action in &self.cleanup_actions {
            if let Some(capability) = action.required_capability() {
                self.require_declared(capability)?;
            }
        }
        for condition in self
            .entry_conditions
            .iter()
            .chain(&self.completion_conditions)
            .chain(&self.abort_conditions)
        {
            if let Some(capability) = condition.required_capability() {
                self.require_declared(capability)?;
            }
        }
        Ok(())
    }

    fn require_declared(&self, capability: MissionCapability) -> Result<(), ValidationError> {
        if self.required_capabilities.contains(&capability) {
            return Ok(());
        }
        Err(ValidationError::UndeclaredCapability {
            phase_id: self.id.clone(),
            capability,
        })
    }
}

impl ExecutionPolicy {
    fn validate(&self) -> Result<(), ValidationError> {
        validation::nonzero_u64(
            "mission.execution_policy.receipt_timeout_ns",
            self.receipt_timeout_ns,
        )
    }
}

fn validate_condition_list(
    field: &str,
    name: &str,
    conditions: &[MissionCondition],
) -> Result<(), ValidationError> {
    validation::count_with_limit(
        &format!("{field}.{name}"),
        conditions.len(),
        MAX_PHASE_CONDITIONS,
    )?;
    for (index, condition) in conditions.iter().enumerate() {
        condition.validate(&format!("{field}.{name}[{index}]"))?;
    }
    Ok(())
}
