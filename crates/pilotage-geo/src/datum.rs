//! Datum discipline: WGS-84 and every horizontal/vertical reference typed, with
//! no bare altitude anywhere.
//!
//! A height is meaningless without its reference, so [`VerticalPosition`] always
//! carries a [`VerticalDatum`] (and, for geometric MSL, a declared
//! [`GeoidModelId`]; for a local-relative height, a [`LocalOriginId`]). A
//! horizontal position always carries a [`HorizontalDatum`]. Unknown datums are
//! refused, never guessed.
//!
//! # Mapping to instrument-state `AltitudeClass`
//!
//! This crate is foundational and `pilotage-instrument-state` is a consumer, so
//! it cannot depend on this crate's inverse; the vertical vocabulary is minted
//! here with an explicit mapping instead of a dependency:
//!
//! | [`VerticalDatum`] | `AltitudeClass` |
//! |---|---|
//! | `Ellipsoid` | (geo-only; no instrument-state class) |
//! | `Msl` | `GeometricMsl` |
//! | `Agl` | `Agl` |
//! | `BaroIndicated` | `BaroIndicated` |
//! | `Pressure` | `Pressure` |
//! | `LocalRelative` | `LocalRelative` |
//! | `Unknown` | `Unknown` |

use crate::error::GeoError;

/// A horizontal geodetic datum. `Unknown` is the fail-closed wire default: a
/// position on an unknown datum has no interpretable frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HorizontalDatum {
    /// Unknown to this build; refused rather than guessed.
    Unknown = 0,
    /// World Geodetic System 1984.
    Wgs84 = 1,
    /// North American Datum 1983.
    Nad83 = 2,
    /// International Terrestrial Reference Frame 2014.
    Itrf2014 = 3,
}

impl HorizontalDatum {
    /// Wire encoding.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Decodes the wire byte, failing closed on an unknown value.
    ///
    /// # Errors
    ///
    /// [`GeoError`] is not returned here; the caller maps `None` to a
    /// fail-closed decode error with the field name.
    #[must_use]
    pub const fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Unknown),
            1 => Some(Self::Wgs84),
            2 => Some(Self::Nad83),
            3 => Some(Self::Itrf2014),
            _ => None,
        }
    }
}

/// A vertical datum: the reference a height is measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VerticalDatum {
    /// Unknown to this build; refused rather than guessed.
    Unknown = 0,
    /// Height above the reference ellipsoid (WGS-84 ellipsoidal height).
    Ellipsoid = 1,
    /// Geometric height above mean sea level, requiring a declared geoid model.
    Msl = 2,
    /// Height above ground level.
    Agl = 3,
    /// Barometric indicated altitude (against a set altimeter datum).
    BaroIndicated = 4,
    /// Pressure altitude (against the standard 1013.25 hPa datum).
    Pressure = 5,
    /// Simulator-local relative height, tied to a declared local origin.
    LocalRelative = 6,
}

impl VerticalDatum {
    /// Wire encoding.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Decodes the wire byte, or `None` for an unknown value.
    #[must_use]
    pub const fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Unknown),
            1 => Some(Self::Ellipsoid),
            2 => Some(Self::Msl),
            3 => Some(Self::Agl),
            4 => Some(Self::BaroIndicated),
            5 => Some(Self::Pressure),
            6 => Some(Self::LocalRelative),
            _ => None,
        }
    }
}

/// Identity of the geoid model behind an MSL separation. `UNDECLARED` (0) means
/// no model was declared, which makes a geometric-MSL height uninterpretable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeoidModelId(pub u16);

impl GeoidModelId {
    /// The sentinel meaning no geoid model was declared.
    pub const UNDECLARED: Self = Self(0);

    /// Whether a geoid model was actually declared.
    #[must_use]
    pub const fn is_declared(self) -> bool {
        self.0 != 0
    }
}

/// Identity of the local origin behind a local-relative height, so an origin
/// rebase is a visible identity change rather than a silent value jump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalOriginId(pub u64);

impl LocalOriginId {
    /// The sentinel meaning no origin was declared.
    pub const UNDECLARED: Self = Self(0);

    /// Whether a local origin was actually declared.
    #[must_use]
    pub const fn is_declared(self) -> bool {
        self.0 != 0
    }
}

/// A height with its vertical reference fully declared. There is no way to
/// construct one without stating the datum (and the geoid or origin it needs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VerticalPosition {
    /// Height in meters, meaningful only against [`Self::datum`].
    pub height_m: f64,
    /// The vertical datum this height is measured against.
    pub datum: VerticalDatum,
    /// Geoid model for an MSL separation; `UNDECLARED` otherwise.
    pub geoid: GeoidModelId,
    /// Local origin for a relative height; `UNDECLARED` otherwise.
    pub origin: LocalOriginId,
}

impl VerticalPosition {
    /// Builds a vertical position, failing closed: the height must be finite,
    /// the datum must be known, a geometric-MSL height needs a declared geoid,
    /// and a local-relative height needs a declared origin.
    ///
    /// # Errors
    ///
    /// A [`GeoError`] describing the missing declaration.
    pub fn new(
        height_m: f64,
        datum: VerticalDatum,
        geoid: GeoidModelId,
        origin: LocalOriginId,
    ) -> Result<Self, GeoError> {
        if !height_m.is_finite() {
            return Err(GeoError::NonFinite { field: "height_m" });
        }
        match datum {
            VerticalDatum::Unknown => return Err(GeoError::UnknownVerticalDatum),
            VerticalDatum::Msl if !geoid.is_declared() => {
                return Err(GeoError::UndeclaredGeoidModel);
            }
            VerticalDatum::LocalRelative if !origin.is_declared() => {
                return Err(GeoError::NonFinite {
                    field: "local_origin_undeclared",
                });
            }
            _ => {}
        }
        Ok(Self {
            height_m,
            datum,
            geoid,
            origin,
        })
    }
}

/// A geodetic position: latitude, longitude, horizontal datum, and a fully
/// declared vertical position. Longitude is normalized to `[-180, 180)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeodeticPosition {
    /// Latitude in degrees, in `[-90, 90]`.
    pub latitude_deg: f64,
    /// Longitude in degrees, normalized to `[-180, 180)`.
    pub longitude_deg: f64,
    /// The horizontal datum.
    pub horizontal_datum: HorizontalDatum,
    /// The vertical position.
    pub vertical: VerticalPosition,
}

/// A discrete geospatial tile index at a fixed tile size, computed by flooring,
/// so a position on a seam maps deterministically to a single tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeoTile {
    /// Tile index along latitude.
    pub lat_index: i32,
    /// Tile index along longitude.
    pub lon_index: i32,
}

/// Normalizes a longitude to `[-180, 180)` by wrapping whole turns, so the
/// anti-meridian has one canonical representation (`+180` wraps to `-180`).
#[must_use]
pub fn wrap_longitude_deg(lon_deg: f64) -> f64 {
    lon_deg - 360.0 * libm::floor((lon_deg + 180.0) / 360.0)
}

impl GeodeticPosition {
    /// Builds a geodetic position, failing closed: coordinates must be finite,
    /// latitude must be in `[-90, 90]`, the horizontal datum must be known, and
    /// the longitude is normalized to `[-180, 180)` so the anti-meridian is
    /// unambiguous.
    ///
    /// # Errors
    ///
    /// A [`GeoError`] for a non-finite coordinate, an out-of-range latitude, or
    /// an unknown horizontal datum.
    pub fn new(
        latitude_deg: f64,
        longitude_deg: f64,
        horizontal_datum: HorizontalDatum,
        vertical: VerticalPosition,
    ) -> Result<Self, GeoError> {
        if !latitude_deg.is_finite() {
            return Err(GeoError::NonFinite {
                field: "latitude_deg",
            });
        }
        if !longitude_deg.is_finite() {
            return Err(GeoError::NonFinite {
                field: "longitude_deg",
            });
        }
        if !(-90.0..=90.0).contains(&latitude_deg) {
            return Err(GeoError::LatitudeOutOfRange {
                lat_deg: latitude_deg,
            });
        }
        if horizontal_datum == HorizontalDatum::Unknown {
            return Err(GeoError::UnknownVerticalDatum);
        }
        Ok(Self {
            latitude_deg,
            longitude_deg: wrap_longitude_deg(longitude_deg),
            horizontal_datum,
            vertical,
        })
    }

    /// Whether this position lies on the anti-meridian (±180°), where longitude
    /// has two spellings for one place.
    #[must_use]
    pub fn on_antimeridian(&self) -> bool {
        // After normalization the anti-meridian is exactly -180.
        self.longitude_deg == -180.0
    }

    /// Whether this position is at a geographic pole, where longitude is
    /// degenerate.
    #[must_use]
    pub fn at_pole(&self) -> bool {
        self.latitude_deg == 90.0 || self.latitude_deg == -90.0
    }

    /// The tile this position falls in at `tile_deg`-degree tiles, computed by
    /// flooring so a seam belongs to exactly one tile. `tile_deg` must be
    /// positive and finite; a non-positive value yields the zero tile.
    #[must_use]
    pub fn tile(&self, tile_deg: f64) -> GeoTile {
        if !(tile_deg.is_finite() && tile_deg > 0.0) {
            return GeoTile {
                lat_index: 0,
                lon_index: 0,
            };
        }
        GeoTile {
            lat_index: libm::floor(self.latitude_deg / tile_deg) as i32,
            lon_index: libm::floor(self.longitude_deg / tile_deg) as i32,
        }
    }
}

#[cfg(test)]
mod tests;
