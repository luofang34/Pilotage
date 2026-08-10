//! Renderer-edge baseline feature representation.

use std::collections::BTreeMap;

use pilotage_airspace_view::{SubjectExtentV1, SubjectFamilyV1, SubjectReferenceV1};
use sha2::{Digest, Sha256};

use crate::config::NavdataTileConfig;
use crate::mercator::WorldPoint;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum LayerKind {
    Airspace,
    Airway,
    Aerodrome,
    Navaid,
    Fix,
}

impl LayerKind {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Airspace => "airspaces",
            Self::Airway => "airways",
            Self::Aerodrome => "aerodromes",
            Self::Navaid => "navaids",
            Self::Fix => "fixes",
        }
    }

    pub(crate) const fn min_zoom(self, config: NavdataTileConfig) -> u8 {
        match self {
            Self::Airspace => config.airspace_min_zoom,
            Self::Airway => config.airway_min_zoom,
            Self::Aerodrome => config.aerodrome_min_zoom,
            Self::Navaid => config.navaid_min_zoom,
            Self::Fix => config.fix_min_zoom,
        }
    }

    pub(crate) const fn all() -> [Self; 5] {
        [
            Self::Airspace,
            Self::Airway,
            Self::Aerodrome,
            Self::Navaid,
            Self::Fix,
        ]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BaselineGeometry {
    Point(WorldPoint),
    Lines(Vec<Vec<WorldPoint>>),
    Polygon(Vec<WorldPoint>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BaselineFeature {
    pub(crate) layer: LayerKind,
    pub(crate) feature_id: u64,
    pub(crate) properties: BTreeMap<String, String>,
    pub(crate) geometry: BaselineGeometry,
}

pub(crate) fn standard_properties(
    cycle: &str,
    family: SubjectFamilyV1,
    identifier: &str,
    parent: Option<&str>,
    region: Option<&str>,
    name: Option<&str>,
) -> BTreeMap<String, String> {
    let subject = SubjectReferenceV1 {
        cycle: cycle.to_owned(),
        family,
        identifier: identifier.to_owned(),
        parent_identifier: parent.map(str::to_owned),
        region: region.map(str::to_owned),
        extent: SubjectExtentV1::Whole,
    };
    properties(cycle, identifier, name, subject.stable_subject_id())
}

pub(crate) fn airway_properties(
    cycle: &str,
    identifier: &str,
    location: &str,
) -> BTreeMap<String, String> {
    let stable_id = stable_subject_id("airway", Some(location), identifier, None);
    let mut values = properties(cycle, identifier, None, stable_id);
    values.insert("location".to_owned(), location.to_owned());
    values
}

fn properties(
    cycle: &str,
    identifier: &str,
    name: Option<&str>,
    stable_id: String,
) -> BTreeMap<String, String> {
    let mut values = BTreeMap::from([
        ("identifier".to_owned(), identifier.to_owned()),
        ("subject_cycle".to_owned(), cycle.to_owned()),
        ("subject_id".to_owned(), stable_id),
    ]);
    if let Some(name) = name.filter(|value| !value.trim().is_empty()) {
        values.insert("name".to_owned(), name.to_owned());
    }
    values
}

fn stable_subject_id(
    family: &str,
    parent: Option<&str>,
    identifier: &str,
    region: Option<&str>,
) -> String {
    let parent = canonical_ident(parent.unwrap_or(""));
    let identifier = canonical_ident(identifier);
    let region = canonical_ident(region.unwrap_or(""));
    format!(
        "subject-v1|{family}|{}:{parent}|{}:{identifier}|{}:{region}",
        parent.len(),
        identifier.len(),
        region.len()
    )
}

fn canonical_ident(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

pub(crate) fn feature_id(stable_id: &str, discriminator: &str) -> u64 {
    let mut digest = Sha256::new();
    digest.update(stable_id.as_bytes());
    digest.update([0]);
    digest.update(discriminator.as_bytes());
    let bytes = digest.finalize();
    u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}
