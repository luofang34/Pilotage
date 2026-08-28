//! Mission and referenced-artifact identities.

use serde::{Deserialize, Serialize};

use crate::{Digest, ValidationError, validation};

/// The identity of one navigation-data snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationDataIdentity {
    /// The navigation-data cycle.
    pub cycle: String,
    /// The immutable snapshot identifier.
    pub snapshot_id: String,
    /// The snapshot content digest.
    pub snapshot_digest: Digest,
}

/// The identity of one mission document revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionIdentity {
    /// The immutable mission revision identifier.
    pub revision_id: String,
    /// The mission document schema version.
    pub schema_version: u16,
    /// The mission content digest.
    pub content_digest: Digest,
    /// The navigation data that applies to the mission.
    pub navigation_data_identity: NavigationDataIdentity,
}

/// A reference to one immutable flight plan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightPlanReference {
    /// The immutable plan identifier.
    pub plan_id: String,
    /// The plan content digest.
    pub plan_content_digest: Digest,
    /// The navigation data used to resolve the plan.
    pub navigation_data_identity: NavigationDataIdentity,
}

/// The identity of one immutable trial artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactIdentity {
    /// The stable artifact identifier.
    pub id: String,
    /// The artifact revision identifier.
    pub revision: String,
    /// The artifact content digest.
    pub digest: Digest,
}

impl NavigationDataIdentity {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        validation::text(&format!("{field}.cycle"), &self.cycle)?;
        validation::text(&format!("{field}.snapshot_id"), &self.snapshot_id)?;
        validation::digest(&format!("{field}.snapshot_digest"), self.snapshot_digest)
    }
}

impl MissionIdentity {
    pub(crate) fn validate_fields(&self) -> Result<(), ValidationError> {
        self.validate_content_fields()?;
        validation::digest("mission.identity.content_digest", self.content_digest)
    }

    pub(crate) fn validate_content_fields(&self) -> Result<(), ValidationError> {
        validation::text("mission.identity.revision_id", &self.revision_id)?;
        validation::schema(self.schema_version)?;
        self.navigation_data_identity
            .validate("mission.identity.navigation_data_identity")
    }
}

impl FlightPlanReference {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        validation::text(&format!("{field}.plan_id"), &self.plan_id)?;
        validation::digest(
            &format!("{field}.plan_content_digest"),
            self.plan_content_digest,
        )?;
        self.navigation_data_identity
            .validate(&format!("{field}.navigation_data_identity"))
    }
}

impl ArtifactIdentity {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        validation::text(&format!("{field}.id"), &self.id)?;
        validation::text(&format!("{field}.revision"), &self.revision)?;
        validation::digest(&format!("{field}.digest"), self.digest)
    }
}
