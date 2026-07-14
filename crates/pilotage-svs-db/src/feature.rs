//! The aeronautical feature classes a database package can carry, and a set of
//! them.
//!
//! A package declares which classes it covers ([`FeatureSet`]); a tile belongs
//! to exactly one class ([`FeatureClass`]). The class is part of a tile's
//! identity, so a terrain tile and an obstacle tile at the same geographic index
//! are distinct leaves under the tile-root hash and can never be substituted for
//! one another.

/// One aeronautical feature class. The discriminant is the wire encoding and
/// also the canonical ordering key, so tiles sort deterministically by class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum FeatureClass {
    /// Terrain elevation posts.
    Terrain = 1,
    /// Vertical obstacles (towers, masts, buildings).
    Obstacles = 2,
    /// Aerodrome reference points and layouts.
    Aerodromes = 3,
    /// Runway geometry.
    Runways = 4,
    /// Taxiway geometry.
    Taxiways = 5,
}

impl FeatureClass {
    /// Every class, in canonical order.
    pub const ALL: [Self; 5] = [
        Self::Terrain,
        Self::Obstacles,
        Self::Aerodromes,
        Self::Runways,
        Self::Taxiways,
    ];

    /// Wire encoding.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Decodes the wire byte, or `None` for a value outside the known set.
    #[must_use]
    pub const fn from_u8(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Terrain),
            2 => Some(Self::Obstacles),
            3 => Some(Self::Aerodromes),
            4 => Some(Self::Runways),
            5 => Some(Self::Taxiways),
            _ => None,
        }
    }

    /// The single-bit mask this class occupies in a [`FeatureSet`].
    #[must_use]
    const fn mask(self) -> u32 {
        1u32 << (self as u32)
    }
}

/// A set of feature classes a package declares it covers, as a bitset. The bit
/// for a class is its [`FeatureClass::to_u8`] position, so the encoding is
/// stable and independent of insertion order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureSet(u32);

impl FeatureSet {
    /// The empty set.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// This set with `class` added.
    #[must_use]
    pub const fn with(self, class: FeatureClass) -> Self {
        Self(self.0 | class.mask())
    }

    /// Whether `class` is present.
    #[must_use]
    pub const fn contains(self, class: FeatureClass) -> bool {
        self.0 & class.mask() != 0
    }

    /// Whether the set is empty — a package covering no class is refused.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The raw bits, for the canonical serialization.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Rebuilds a set from raw bits that carry no unknown class positions, or
    /// `None` when a bit outside the known classes is set (fail closed rather
    /// than silently accepting an unrecognized class).
    #[must_use]
    pub const fn from_bits(bits: u32) -> Option<Self> {
        let mut known = 0u32;
        let mut i = 0;
        while i < FeatureClass::ALL.len() {
            known |= FeatureClass::ALL[i].mask();
            i += 1;
        }
        if bits & !known != 0 {
            None
        } else {
            Some(Self(bits))
        }
    }
}
