//! Versioned input and result types for AirspaceView resolution.

use aerocontext_core::{Area, GeoPoint, NavDataSnapshot};
use serde::{Deserialize, Serialize};

use crate::AirspaceViewError;

/// Identity of one immutable Navdata snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavdataIdentityV1 {
    /// Authority and effective date for the cycle.
    pub cycle: String,
    /// Identity of the immutable built snapshot.
    pub snapshot_id: String,
    /// Digest of the canonical snapshot content and cycle.
    pub snapshot_digest: String,
}

/// One immutable Navdata snapshot with its required identity.
#[derive(Debug, Clone, PartialEq)]
pub struct IdentifiedNavdataSnapshotV1 {
    identity: NavdataIdentityV1,
    snapshot: NavDataSnapshot,
}

impl IdentifiedNavdataSnapshotV1 {
    /// Makes an identified snapshot after it verifies the cycle identity.
    ///
    /// # Errors
    ///
    /// Returns [`AirspaceViewError::SnapshotCycleMismatch`] when the supplied
    /// cycle does not identify `snapshot`.
    pub fn try_new(
        identity: NavdataIdentityV1,
        snapshot: NavDataSnapshot,
    ) -> Result<Self, AirspaceViewError> {
        let snapshot_cycle = navdata_cycle_id(&snapshot);
        if identity.cycle != snapshot_cycle {
            return Err(AirspaceViewError::SnapshotCycleMismatch {
                identity_cycle: identity.cycle,
                snapshot_cycle,
            });
        }
        Ok(Self { identity, snapshot })
    }

    /// Gets the complete Navdata identity.
    #[must_use]
    pub const fn identity(&self) -> &NavdataIdentityV1 {
        &self.identity
    }

    pub(crate) const fn snapshot(&self) -> &NavDataSnapshot {
        &self.snapshot
    }
}

/// Makes the stable cycle identity used by this contract.
#[must_use]
pub fn navdata_cycle_id(snapshot: &NavDataSnapshot) -> String {
    format!(
        "{}:{}",
        snapshot.cycle.authority.slug(),
        snapshot.cycle.effective_on
    )
}

/// Family of a subject named by an aeronautical update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectFamilyV1 {
    /// Airport or another landing facility.
    Aerodrome,
    /// Ground-based navigation aid.
    Navaid,
    /// Published fix or waypoint.
    Fix,
    /// One runway at an aerodrome.
    Runway,
    /// One controlled or special-use airspace subject.
    Airspace,
    /// Instrument or other published procedure.
    Procedure,
    /// Aeronautical service or frequency.
    Service,
    /// A family that this schema does not name.
    Other,
}

impl SubjectFamilyV1 {
    pub(crate) const fn slug(self) -> &'static str {
        match self {
            Self::Aerodrome => "aerodrome",
            Self::Navaid => "navaid",
            Self::Fix => "fix",
            Self::Runway => "runway",
            Self::Airspace => "airspace",
            Self::Procedure => "procedure",
            Self::Service => "service",
            Self::Other => "other",
        }
    }
}

/// Extent of the named subject that an update changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SubjectExtentV1 {
    /// The update changes the complete subject.
    Whole,
    /// The update changes one measured runway segment.
    RunwaySegment {
        /// Runway end from which the offsets are measured.
        from_end: String,
        /// Offset from the named end, in feet.
        start_offset_ft: u32,
        /// Length of the changed segment, in feet.
        length_ft: u32,
    },
    /// The update changes one facility component.
    FacilityComponent {
        /// Published component name.
        component: String,
    },
    /// The source states another partial extent.
    OtherPartial {
        /// Source-neutral description of the extent.
        description: String,
    },
}

/// A cycle-scoped reference to one baseline subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectReferenceV1 {
    /// Navdata cycle in which the identifier has meaning.
    pub cycle: String,
    /// Subject family.
    pub family: SubjectFamilyV1,
    /// Published subject identifier.
    pub identifier: String,
    /// Parent identifier, such as the aerodrome for a runway.
    pub parent_identifier: Option<String>,
    /// Authority region that disambiguates a navigation point.
    pub region: Option<String>,
    /// Part of the subject that the update changes.
    pub extent: SubjectExtentV1,
}

impl SubjectReferenceV1 {
    /// Makes the renderer-edge identifier for the baseline subject.
    ///
    /// The cycle stays outside this value. A composition compares cycle
    /// identities before it uses the same subject identifier in two inputs.
    #[must_use]
    pub fn stable_subject_id(&self) -> String {
        let parent = canonical_ident(self.parent_identifier.as_deref().unwrap_or(""));
        let identifier = canonical_ident(&self.identifier);
        let region = canonical_ident(self.region.as_deref().unwrap_or(""));
        format!(
            "subject-v1|{}|{}:{}|{}:{}|{}:{}",
            self.family.slug(),
            parent.len(),
            parent,
            identifier.len(),
            identifier,
            region.len(),
            region
        )
    }
}

/// Stable baseline subject identity with its Navdata cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectIdentityV1 {
    /// Navdata cycle in which the stable identifier has meaning.
    pub cycle: String,
    /// Stable identifier within the named cycle.
    pub stable_id: String,
}

impl From<&SubjectReferenceV1> for SubjectIdentityV1 {
    fn from(subject: &SubjectReferenceV1) -> Self {
        Self {
            cycle: subject.cycle.clone(),
            stable_id: subject.stable_subject_id(),
        }
    }
}

fn canonical_ident(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

/// Source-neutral horizontal geometry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum GeometryV1 {
    /// One WGS84 position.
    Point {
        /// Published position.
        position: GeoPoint,
    },
    /// One WGS84 line segment.
    Line {
        /// First endpoint.
        start: GeoPoint,
        /// Second endpoint.
        end: GeoPoint,
    },
    /// One horizontal area.
    Area {
        /// Published area.
        area: Area,
    },
}

/// Amount of a subject described by one geometry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum GeometryCoverageV1 {
    /// Geometry describes the complete subject.
    WholeSubject,
    /// Geometry describes only the stated extent.
    Partial {
        /// Exact partial extent represented by the geometry.
        extent: SubjectExtentV1,
    },
}

/// Geometry carried directly by an update.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateGeometryV1 {
    /// Horizontal geometry.
    pub geometry: GeometryV1,
    /// Subject extent represented by the geometry.
    pub coverage: GeometryCoverageV1,
}

/// One source-neutral aeronautical update.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AeronauticalUpdateV1 {
    /// Stable update identity.
    pub update_id: String,
    /// Complete display text for the non-map surface.
    pub display_text: String,
    /// Baseline subject, when the update names one.
    pub subject: Option<SubjectReferenceV1>,
    /// Direct geometry, when the update supplies it.
    pub geometry: Option<UpdateGeometryV1>,
}

/// Typed reason that geometry resolution did not supply geometry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "reason")]
pub enum ResolutionFailureReasonV1 {
    /// The identifier belongs to a different Navdata cycle.
    IdentifierFromAnotherCycle {
        /// Cycle named by the update.
        subject_cycle: String,
        /// Cycle used by the resolver.
        snapshot_cycle: String,
    },
    /// The snapshot has no matching identifier in the selected family.
    UnknownIdentifier {
        /// Selected subject family.
        family: SubjectFamilyV1,
        /// Identifier that did not match.
        identifier: String,
    },
    /// The identifier matches more than one baseline subject.
    AmbiguousMatch {
        /// Selected subject family.
        family: SubjectFamilyV1,
        /// Identifier that matched more than once.
        identifier: String,
        /// Number of matches.
        matches: u32,
    },
    /// The Navdata snapshot does not contain this subject family.
    SubjectFamilyNotCarried {
        /// Family that is not carried.
        family: SubjectFamilyV1,
    },
    /// The snapshot contains the subject but not useful horizontal geometry.
    GeometryNotCarried {
        /// Subject family.
        family: SubjectFamilyV1,
        /// Identifier of the subject without geometry.
        identifier: String,
    },
    /// The snapshot cannot make geometry for the stated partial extent.
    PartialGeometryNotCarried {
        /// Subject family.
        family: SubjectFamilyV1,
        /// Identifier of the partial subject.
        identifier: String,
        /// Partial extent that must not become whole-subject geometry.
        extent: SubjectExtentV1,
    },
    /// Direct geometry describes a different extent from the named subject.
    DirectGeometryExtentMismatch {
        /// Extent named by the update subject.
        subject_extent: SubjectExtentV1,
        /// Extent declared for the direct geometry.
        geometry_coverage: GeometryCoverageV1,
    },
}

/// Geometry produced for one update.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedGeometryV1 {
    /// Horizontal geometry.
    pub geometry: GeometryV1,
    /// Subject extent represented by the geometry.
    pub coverage: GeometryCoverageV1,
}

/// How an update acquired, or did not acquire, geometry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum GeometryResolutionV1 {
    /// The update supplied its own geometry. No lookup occurred.
    Direct,
    /// AirspaceView resolved the subject against the named snapshot.
    ResolvedFromNavdata,
    /// Baseline resolution failed for a typed reason.
    Unresolved {
        /// Reason that geometry is absent.
        reason: ResolutionFailureReasonV1,
    },
    /// The update names no subject geometry.
    NoSubjectGeometry,
}

/// One update in the derived AirspaceView result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AirspaceViewItemV1 {
    /// Stable update identity.
    pub update_id: String,
    /// Complete text for the required non-map surface.
    pub display_text: String,
    /// Cycle-scoped baseline subject identity, when a subject exists.
    pub subject_identity: Option<SubjectIdentityV1>,
    /// Geometry when direct input or baseline resolution supplied it.
    pub geometry: Option<ResolvedGeometryV1>,
    /// Resolution disposition.
    pub resolution: GeometryResolutionV1,
}

/// Contract statement about the role of the map surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "role")]
pub enum MapCompletenessV1 {
    /// The map is supplemental. A list must show every update.
    SupplementalOnly {
        /// Number of result items that have no geometry.
        updates_without_geometry: u32,
    },
}

/// Complete derived result for one Navdata snapshot and update set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AirspaceViewResultV1 {
    /// Contract schema version.
    pub schema_version: u16,
    /// Identity of the one snapshot used for all resolution.
    pub navdata_identity: NavdataIdentityV1,
    /// Every input update in input order.
    pub updates: Vec<AirspaceViewItemV1>,
    /// The map is never the only update surface.
    pub map_completeness: MapCompletenessV1,
}
