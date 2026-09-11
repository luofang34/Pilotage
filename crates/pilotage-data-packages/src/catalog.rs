use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::{Channel, PackageError, PackageId, Product, Release, error::invalid};

/// A publisher's available immutable releases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    /// Catalog contract version.
    pub schema_version: u32,
    /// Publisher sequence used to detect metadata rollback.
    pub sequence: u64,
    /// Catalog issue time in Unix seconds.
    pub generated_at: i64,
    /// Metadata expiry in Unix seconds.
    pub expires_at: i64,
    /// Available product releases.
    pub releases: Vec<Release>,
}

/// Signed catalog bytes. Verification precedes JSON decoding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedCatalog {
    /// Trusted publisher key name.
    pub key_id: String,
    /// Exact UTF-8 JSON bytes that the publisher signed.
    pub payload: String,
    /// Ed25519 signature bytes.
    pub signature: Vec<u8>,
}

/// Trust and freshness requirements supplied by the composition host.
pub struct CatalogTrust {
    /// Trusted public keys, indexed by publisher key name.
    pub keys: BTreeMap<String, [u8; 32]>,
    /// Smallest acceptable publisher sequence.
    pub minimum_sequence: u64,
    /// Evaluation time in Unix seconds.
    pub now: i64,
}

impl SignedCatalog {
    /// Authenticate and validate metadata before it can supply updates.
    pub fn verify(&self, trust: &CatalogTrust) -> Result<Catalog, PackageError> {
        let bytes = trust
            .keys
            .get(&self.key_id)
            .ok_or_else(|| invalid("signing_key", &self.key_id))?;
        let key = VerifyingKey::from_bytes(bytes).map_err(|source| self.signature_error(source))?;
        let signature = Signature::from_slice(&self.signature)
            .map_err(|source| self.signature_error(source))?;
        key.verify_strict(self.payload.as_bytes(), &signature)
            .map_err(|source| self.signature_error(source))?;
        let catalog: Catalog = serde_json::from_str(&self.payload)?;
        catalog.validate()?;
        if catalog.sequence < trust.minimum_sequence {
            return Err(invalid("catalog_sequence", catalog.sequence));
        }
        if catalog.generated_at > trust.now || catalog.expires_at <= trust.now {
            return Err(invalid("catalog_validity", trust.now));
        }
        Ok(catalog)
    }

    fn signature_error(&self, source: ed25519_dalek::SignatureError) -> PackageError {
        PackageError::Signature {
            key_id: self.key_id.clone(),
            source,
        }
    }
}

impl Catalog {
    /// Validate releases and their exact dependency graph.
    pub fn validate(&self) -> Result<(), PackageError> {
        if self.schema_version != 1 {
            return Err(invalid("catalog_schema", self.schema_version));
        }
        if self.generated_at >= self.expires_at {
            return Err(invalid("catalog_validity", "empty"));
        }
        let mut ids = BTreeSet::new();
        for release in &self.releases {
            release.validate()?;
            if !ids.insert(&release.id) {
                return Err(invalid("release_id", release.id.as_str()));
            }
        }
        for release in &self.releases {
            self.visit_dependencies(&release.id, &mut BTreeSet::new(), &mut BTreeSet::new())?;
        }
        Ok(())
    }

    /// Select the current release for one product, provider, and region.
    pub fn current(
        &self,
        product: Product,
        authority: &str,
        region: &str,
        now: i64,
        channel: Channel,
    ) -> Option<&Release> {
        self.releases
            .iter()
            .filter(|r| {
                r.product == product
                    && r.authority == authority
                    && r.coverage.name == region
                    && r.channel == channel
                    && r.valid_at(now)
            })
            .max_by_key(|r| (r.validity.map(|v| v.effective_at), r.revision, &r.id))
    }

    /// Select the next published edition for preparation.
    pub fn upcoming(
        &self,
        product: Product,
        authority: &str,
        region: &str,
        now: i64,
        channel: Channel,
    ) -> Option<&Release> {
        self.releases
            .iter()
            .filter(|r| {
                r.product == product
                    && r.authority == authority
                    && r.coverage.name == region
                    && r.channel == channel
                    && r.validity.is_some_and(|v| v.effective_at > now)
            })
            .min_by_key(|r| {
                (
                    r.validity.map(|v| v.effective_at),
                    std::cmp::Reverse(r.revision),
                    &r.id,
                )
            })
    }

    /// Resolve dependencies before their consumers for installation.
    pub fn installation_order(&self, id: &PackageId) -> Result<Vec<&Release>, PackageError> {
        self.validate()?;
        let mut order = Vec::new();
        self.collect_dependencies(id, &mut BTreeSet::new(), &mut order)?;
        Ok(order)
    }

    fn release(&self, id: &PackageId) -> Result<&Release, PackageError> {
        self.releases
            .iter()
            .find(|r| &r.id == id)
            .ok_or_else(|| PackageError::Missing {
                id: id.as_str().to_owned(),
            })
    }

    fn collect_dependencies<'a>(
        &'a self,
        id: &PackageId,
        seen: &mut BTreeSet<PackageId>,
        order: &mut Vec<&'a Release>,
    ) -> Result<(), PackageError> {
        if !seen.insert(id.clone()) {
            return Ok(());
        }
        let release = self.release(id)?;
        for dependency in &release.dependencies {
            self.collect_dependencies(dependency, seen, order)?;
        }
        order.push(release);
        Ok(())
    }

    fn visit_dependencies(
        &self,
        id: &PackageId,
        visiting: &mut BTreeSet<PackageId>,
        visited: &mut BTreeSet<PackageId>,
    ) -> Result<(), PackageError> {
        if visited.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id.clone()) {
            return Err(invalid("dependency_cycle", id.as_str()));
        }
        for dependency in &self.release(id)?.dependencies {
            self.visit_dependencies(dependency, visiting, visited)?;
        }
        visiting.remove(id);
        visited.insert(id.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests;
