use crate::{
    Artifact, ArtifactFormat, Catalog, Channel, ContentDigest, Coverage, Distribution,
    PackageError, PackageId, PackagePath, Product, Release, Validity, error::invalid,
};

/// Convert an AeroContext catalog into the common installation contract.
///
/// `effective_second_utc` supplies the authority's activation time within a UTC day.
/// Coverage must come from source evidence because the legacy catalog omits it.
/// The result requires normal publisher authentication or explicit local development trust.
pub fn import_acnav_catalog(
    manifest: &aerocontext_navdata::Manifest,
    coverage: &Coverage,
    effective_second_utc: u32,
    channel: Channel,
    sequence: u64,
    metadata_expires_at: i64,
) -> Result<Catalog, PackageError> {
    if manifest.manifest_version != 1 || effective_second_utc >= 86_400 {
        return Err(invalid(
            "legacy_catalog",
            "unsupported schema or activation time",
        ));
    }
    let mut releases = Vec::new();
    for entry in &manifest.entries {
        if entry.blob_format_version > aerocontext_navdata::FORMAT_VERSION {
            return Err(invalid("acnav_format", entry.blob_format_version));
        }
        let digest = ContentDigest::try_from(entry.sha256.clone())?;
        let id = format!(
            "{}-{}-{}",
            entry.authority,
            entry.effective_on,
            digest.as_str()
        );
        let at = |date: chrono::NaiveDate| -> Result<i64, PackageError> {
            let time =
                chrono::NaiveTime::from_num_seconds_from_midnight_opt(effective_second_utc, 0)
                    .ok_or_else(|| invalid("activation_time", effective_second_utc))?;
            Ok(date.and_time(time).and_utc().timestamp())
        };
        releases.push(Release {
            schema_version: 1,
            id: PackageId::try_from(id)?,
            product: Product::Navdata,
            authority: entry.authority.clone(),
            revision: 0,
            edition: entry.airac.clone(),
            source_set: digest.clone(),
            channel,
            validity: Some(Validity {
                effective_at: at(entry.effective_on)?,
                expires_at: at(entry.next_effective_on)?,
            }),
            coverage: coverage.clone(),
            artifacts: vec![Artifact {
                path: PackagePath::try_from("navigation.acnav".to_owned())?,
                source: entry.url.clone(),
                format: ArtifactFormat::Acnav,
                bytes: entry.bytes,
                sha256: digest,
            }],
            dependencies: Vec::new(),
            renderer_capabilities: Vec::new(),
            attributions: vec![format!("Navigation source: {}", entry.authority)],
            distribution: if entry.no_redistribute {
                Distribution::Restricted
            } else {
                Distribution::Unspecified
            },
        });
    }
    let catalog = Catalog {
        schema_version: 1,
        sequence,
        generated_at: manifest.generated_at.timestamp(),
        expires_at: metadata_expires_at,
        releases,
    };
    catalog.validate()?;
    Ok(catalog)
}

#[cfg(test)]
mod tests;
