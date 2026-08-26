//! Which north a reported heading is measured from.

/// Which north a reported heading is measured from.
///
/// A heading is a number and a reference. The map draws in true north, so a
/// magnetic heading drawn as a true one is wrong by the local variation, which
/// is tens of degrees in places. An unstated reference is its own case rather
/// than an assumption of true.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum DisplayHeadingReference {
    /// Measured from true north.
    TrueNorth,
    /// Measured from magnetic north.
    MagneticNorth,
    /// The source stated a reference this display does not know.
    Other,
}

impl From<surveillance_core::HeadingReference> for DisplayHeadingReference {
    fn from(value: surveillance_core::HeadingReference) -> Self {
        match value {
            surveillance_core::HeadingReference::TrueNorth => Self::TrueNorth,
            surveillance_core::HeadingReference::MagneticNorth => Self::MagneticNorth,
            _ => Self::Other,
        }
    }
}
