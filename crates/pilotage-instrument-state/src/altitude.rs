//! Datum-qualified altitude: every displayed altitude carries its
//! reference at the type level, and references are never converted or
//! substituted silently.
//!
//! A conventional barometric profile fails visibly when its source is
//! absent — it never falls back to local NED. The simulator's
//! local-relative altitude stays available but is labelled REL on the
//! tape, so geometric height above a session origin can never read as
//! barometric altitude.

/// Identity of the geoid/undulation model behind a geometric-MSL sample.
/// Zero is "undeclared": a geometric-MSL sample without a declared model
/// fails rather than displaying an unattributed height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeoidModelId(pub u8);

impl GeoidModelId {
    /// No model declared.
    pub const UNDECLARED: Self = Self(0);
}

/// Identity of the local origin behind a relative altitude sample, so an
/// origin rebase is a visible identity change, not a silent value jump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OriginId(pub u32);

/// The reference an altitude value is measured against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AltitudeReference {
    /// Simulator-local height above the identified session origin.
    LocalRelative {
        /// Which origin the height is relative to.
        origin: OriginId,
    },
    /// Barometric indicated altitude with the setting the source applied.
    BaroIndicated {
        /// Setting the source applied, hectopascals.
        applied_hpa: f32,
    },
    /// Pressure altitude (standard atmosphere reference).
    Pressure,
    /// Geometric height above mean sea level per an identified model.
    GeometricMsl {
        /// Which geoid model the height is referenced to.
        model: GeoidModelId,
    },
    /// Height above ground level.
    Agl,
}

/// Reference class without payload, for compatibility checks between a
/// displayed altitude and a pilot selection. Selections are compatible
/// only within the same class — numeric equality across references is
/// meaningless and never compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AltitudeClass {
    /// Simulator-local relative height.
    LocalRelative,
    /// Barometric indicated altitude.
    BaroIndicated,
    /// Pressure altitude.
    Pressure,
    /// Geometric MSL.
    GeometricMsl,
    /// Above ground level.
    Agl,
    /// The wire carried a class this build does not know; the altitude
    /// (or the selection) fails rather than guessing a reference.
    Unknown,
}

impl AltitudeClass {
    /// Fail-closed wire decoding: any byte outside the known set is
    /// [`Self::Unknown`].
    #[must_use]
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::LocalRelative,
            1 => Self::BaroIndicated,
            2 => Self::Pressure,
            3 => Self::GeometricMsl,
            4 => Self::Agl,
            _ => Self::Unknown,
        }
    }

    /// Wire encoding; [`Self::Unknown`] intentionally encodes to a value
    /// that decodes back to `Unknown`.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::LocalRelative => 0,
            Self::BaroIndicated => 1,
            Self::Pressure => 2,
            Self::GeometricMsl => 3,
            Self::Agl => 4,
            Self::Unknown => 255,
        }
    }

    /// The tape label identifying this reference to the pilot.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LocalRelative => "REL",
            Self::BaroIndicated => "BARO",
            Self::Pressure => "STD",
            Self::GeometricMsl => "MSL",
            Self::Agl => "AGL",
            Self::Unknown => "REF",
        }
    }
}

impl AltitudeReference {
    /// The payload-free class of this reference.
    #[must_use]
    pub const fn class(&self) -> AltitudeClass {
        match self {
            Self::LocalRelative { .. } => AltitudeClass::LocalRelative,
            Self::BaroIndicated { .. } => AltitudeClass::BaroIndicated,
            Self::Pressure => AltitudeClass::Pressure,
            Self::GeometricMsl { .. } => AltitudeClass::GeometricMsl,
            Self::Agl => AltitudeClass::Agl,
        }
    }
}

/// The feeder's altitude declaration: which reference its primary
/// altitude is measured against, plus the sample and identities the
/// non-local classes need. The local-relative class ignores the sample
/// and derives its value from NED position, as before.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AltitudeDeclaration {
    /// Declared reference class of the primary altitude.
    pub reference_class: AltitudeClass,
    /// Altitude sample in meters for the non-local classes; `None` when
    /// the source is absent (the class then fails, never falls back).
    pub sample_m: Option<f32>,
    /// Geoid model identity for geometric MSL.
    pub geoid_model: GeoidModelId,
    /// Local-origin identity for relative altitude.
    pub origin: OriginId,
}

impl Default for AltitudeDeclaration {
    fn default() -> Self {
        Self {
            reference_class: AltitudeClass::LocalRelative,
            sample_m: None,
            geoid_model: GeoidModelId::UNDECLARED,
            origin: OriginId(0),
        }
    }
}

#[cfg(test)]
mod tests;
