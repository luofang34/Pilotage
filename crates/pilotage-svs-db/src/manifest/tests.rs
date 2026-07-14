//! Tests for the manifest schema, its compatibility policy, and datum
//! discipline.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use pilotage_geo::{DatumRealizationId, HorizontalDatum, VerticalDatum};

use super::{MANIFEST_SCHEMA_VERSION, MIN_SUPPORTED_SCHEMA, schema_is_compatible};
use crate::error::DbError;
use crate::fixtures;

/// Acceptance: the manifest schema is versioned with an explicit compatibility
/// policy closed at both ends — the current version is accepted, and both a
/// newer and an older-than-minimum version are refused rather than guessed.
#[test]
fn manifest_schema_versioned_with_compatibility_policy() {
    assert!(schema_is_compatible(MANIFEST_SCHEMA_VERSION));
    assert!(!schema_is_compatible(MANIFEST_SCHEMA_VERSION + 1));
    assert!(!schema_is_compatible(MIN_SUPPORTED_SCHEMA - 1));

    let candidate = fixtures::candidate();
    let mut newer = candidate.manifest.clone();
    newer.schema_version = MANIFEST_SCHEMA_VERSION + 1;
    assert_eq!(
        newer.validate_structure(),
        Err(DbError::IncompatibleManifest {
            version: MANIFEST_SCHEMA_VERSION + 1,
            min: MIN_SUPPORTED_SCHEMA,
            max: MANIFEST_SCHEMA_VERSION,
        })
    );
}

#[test]
fn default_fixture_manifest_is_structurally_valid() {
    let candidate = fixtures::candidate();
    assert_eq!(candidate.manifest.validate_structure(), Ok(()));
}

#[test]
fn empty_feature_set_is_refused() {
    let candidate = fixtures::candidate();
    let mut manifest = candidate.manifest.clone();
    manifest.content.features = crate::feature::FeatureSet::empty();
    assert_eq!(manifest.validate_structure(), Err(DbError::EmptyFeatureSet));
}

#[test]
fn degenerate_coverage_is_refused() {
    let candidate = fixtures::candidate();
    let mut manifest = candidate.manifest.clone();
    manifest.coverage.region.max_lat_deg = manifest.coverage.region.min_lat_deg;
    assert!(matches!(
        manifest.validate_structure(),
        Err(DbError::InvalidCoverage { .. })
    ));
}

#[test]
fn misordered_effectivity_is_refused() {
    let candidate = fixtures::candidate();
    let mut manifest = candidate.manifest.clone();
    manifest.effectivity.expiry = crate::identity::DayNumber(50);
    assert_eq!(manifest.validate_structure(), Err(DbError::BadEffectivity));
}

#[test]
fn unknown_horizontal_datum_is_refused() {
    let candidate = fixtures::candidate();
    let mut manifest = candidate.manifest.clone();
    manifest.coverage.horizontal_datum = HorizontalDatum::Unknown;
    assert_eq!(
        manifest.validate_structure(),
        Err(DbError::UnknownHorizontalDatum)
    );
}

#[test]
fn realization_bearing_datum_without_realization_is_refused() {
    let candidate = fixtures::candidate();
    let mut manifest = candidate.manifest.clone();
    manifest.coverage.horizontal_datum = HorizontalDatum::Nad83;
    manifest.coverage.realization = DatumRealizationId::UNDECLARED;
    assert_eq!(
        manifest.validate_structure(),
        Err(DbError::UndeclaredRealization)
    );
}

#[test]
fn unknown_vertical_datum_is_refused() {
    let candidate = fixtures::candidate();
    let mut manifest = candidate.manifest.clone();
    manifest.coverage.vertical_datum = VerticalDatum::Unknown;
    assert_eq!(
        manifest.validate_structure(),
        Err(DbError::UnknownVerticalDatum)
    );
}

#[test]
fn geometric_msl_without_geoid_is_refused() {
    let candidate = fixtures::candidate();
    let mut manifest = candidate.manifest.clone();
    manifest.coverage.vertical_datum = VerticalDatum::Msl;
    assert_eq!(manifest.validate_structure(), Err(DbError::UndeclaredGeoid));
}

#[test]
fn active_id_reflects_the_manifest() {
    let candidate = fixtures::candidate();
    let id = candidate.manifest.active_id();
    assert_eq!(id.dataset, candidate.manifest.provenance.dataset);
    assert_eq!(id.version, candidate.manifest.provenance.version);
    assert_eq!(id.simulation_only, candidate.manifest.simulation_only);
}
