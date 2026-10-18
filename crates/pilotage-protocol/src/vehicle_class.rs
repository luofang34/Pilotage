//! The kind of vehicle an adapter declares (`pilotage.v1.VehicleClass`).
//! A client chooses its default control scheme from it.

use crate::wire;

/// The kind of vehicle, as the adapter declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VehicleClass {
    /// A rotorcraft with more than one rotor that can hover.
    Multicopter,
    /// An aircraft that gets lift from a fixed wing and cannot hover.
    FixedWing,
    /// An aircraft that takes off and lands vertically and flies on a wing.
    Vtol,
    /// A vehicle that drives on the ground.
    GroundRover,
    /// A vehicle that moves on the water surface.
    SurfaceVessel,
}

impl VehicleClass {
    /// The wire enum for this class.
    #[must_use]
    pub const fn to_wire(self) -> wire::VehicleClass {
        match self {
            Self::Multicopter => wire::VehicleClass::Multicopter,
            Self::FixedWing => wire::VehicleClass::FixedWing,
            Self::Vtol => wire::VehicleClass::Vtol,
            Self::GroundRover => wire::VehicleClass::GroundRover,
            Self::SurfaceVessel => wire::VehicleClass::SurfaceVessel,
        }
    }

    /// The class a raw wire value carries. `None` for
    /// `VEHICLE_CLASS_UNSPECIFIED` and for a value this build does not
    /// know.
    #[must_use]
    pub fn from_wire(raw: i32) -> Option<Self> {
        match wire::VehicleClass::try_from(raw).ok()? {
            wire::VehicleClass::Unspecified => None,
            wire::VehicleClass::Multicopter => Some(Self::Multicopter),
            wire::VehicleClass::FixedWing => Some(Self::FixedWing),
            wire::VehicleClass::Vtol => Some(Self::Vtol),
            wire::VehicleClass::GroundRover => Some(Self::GroundRover),
            wire::VehicleClass::SurfaceVessel => Some(Self::SurfaceVessel),
        }
    }
}

#[cfg(test)]
mod tests;
