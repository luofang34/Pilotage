//! Which north a heading is measured from (`pilotage.v1.HeadingReference`)
//! and where a magnetic variation comes from
//! (`pilotage.v1.MagneticVariationSource`).

use crate::wire;

/// Which north a vehicle's attitude yaw is measured from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeadingReference {
    /// True (geographic) north.
    True,
    /// Magnetic north.
    Magnetic,
}

impl HeadingReference {
    /// The wire enum for this reference.
    #[must_use]
    pub const fn to_wire(self) -> wire::HeadingReference {
        match self {
            Self::True => wire::HeadingReference::True,
            Self::Magnetic => wire::HeadingReference::Magnetic,
        }
    }

    /// The wire value for an optional reference: `UNSPECIFIED` for `None`.
    #[must_use]
    pub const fn option_to_wire(reference: Option<Self>) -> wire::HeadingReference {
        match reference {
            Some(reference) => reference.to_wire(),
            None => wire::HeadingReference::Unspecified,
        }
    }

    /// The reference a raw wire value carries. `None` for
    /// `HEADING_REFERENCE_UNSPECIFIED` and for a value this build does not
    /// know.
    #[must_use]
    pub fn from_wire(raw: i32) -> Option<Self> {
        match wire::HeadingReference::try_from(raw).ok()? {
            wire::HeadingReference::Unspecified => None,
            wire::HeadingReference::True => Some(Self::True),
            wire::HeadingReference::Magnetic => Some(Self::Magnetic),
        }
    }
}

/// Where a magnetic variation value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MagneticVariationSource {
    /// The flight controller's own geomagnetic model at its position.
    FcModel,
    /// A value configured on the flight controller.
    FcConfigured,
    /// A geomagnetic model shared by the host and its clients.
    SharedModel,
    /// A published aeronautical chart value.
    PublishedChart,
    /// A value the operator entered.
    Operator,
    /// The vehicle applies no variation; its attitude yaw is magnetic.
    None,
}

impl MagneticVariationSource {
    /// The wire enum for this source.
    #[must_use]
    pub const fn to_wire(self) -> wire::MagneticVariationSource {
        match self {
            Self::FcModel => wire::MagneticVariationSource::FcModel,
            Self::FcConfigured => wire::MagneticVariationSource::FcConfigured,
            Self::SharedModel => wire::MagneticVariationSource::SharedModel,
            Self::PublishedChart => wire::MagneticVariationSource::PublishedChart,
            Self::Operator => wire::MagneticVariationSource::Operator,
            Self::None => wire::MagneticVariationSource::None,
        }
    }

    /// The source a raw wire value carries. `None` (no value) for
    /// `MAGNETIC_VARIATION_SOURCE_UNSPECIFIED` and for a value this build
    /// does not know.
    #[must_use]
    pub fn from_wire(raw: i32) -> Option<Self> {
        match wire::MagneticVariationSource::try_from(raw).ok()? {
            wire::MagneticVariationSource::Unspecified => None,
            wire::MagneticVariationSource::FcModel => Some(Self::FcModel),
            wire::MagneticVariationSource::FcConfigured => Some(Self::FcConfigured),
            wire::MagneticVariationSource::SharedModel => Some(Self::SharedModel),
            wire::MagneticVariationSource::PublishedChart => Some(Self::PublishedChart),
            wire::MagneticVariationSource::Operator => Some(Self::Operator),
            wire::MagneticVariationSource::None => Some(Self::None),
        }
    }

    /// The north an attitude yaw is measured from when its vehicle states
    /// this source: magnetic for `None`, true for every source that states
    /// a declination the vehicle applies.
    #[must_use]
    pub const fn heading_reference(self) -> HeadingReference {
        match self {
            Self::None => HeadingReference::Magnetic,
            _ => HeadingReference::True,
        }
    }
}

#[cfg(test)]
mod tests;
