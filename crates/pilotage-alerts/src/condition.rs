//! The typed alert-condition vocabulary and the stable identities alerts
//! carry.
//!
//! Producers hand the manager typed conditions, never raw telemetry. Each
//! future monitor owns a fault sub-vocabulary here: altitude reference,
//! heading/navigation reference, turn/slip, source miscompare, display and
//! renderer health, frame identity, and non-flight system notes. Every
//! condition maps deterministically to a stable [`AlertId`] and a fixed
//! [`AlertClass`], so a fault's identity and severity are properties of the
//! fault, not of the frame or the airframe.
//!
//! Identities pack a one-byte family selector and a one-byte fault code:
//! `id = (family << 8) | code`. [`class_of`] reverses an id back to its
//! class, which the profile uses to reject inhibiting an uninhibitable
//! alert.

use crate::class::AlertClass;

/// Stable, machine-readable alert identity. Ordering is defined so ties in
/// severity break by ascending id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AlertId(pub u16);

const FAMILY_ALTITUDE: u8 = 0x01;
const FAMILY_NAV: u8 = 0x02;
const FAMILY_TURN_SLIP: u8 = 0x03;
const FAMILY_MISCOMPARE: u8 = 0x04;
const FAMILY_DISPLAY: u8 = 0x05;
const FAMILY_FRAME: u8 = 0x06;
const FAMILY_SYSTEM: u8 = 0x07;

const fn id(family: u8, code: u8) -> AlertId {
    AlertId(((family as u16) << 8) | code as u16)
}

/// Altitude reference/datum faults (from the altitude monitor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AltFault {
    /// Barometric reference or datum lost; the altitude datum is untrusted.
    ReferenceLost,
    /// Two altitude datums disagree beyond tolerance.
    DatumMiscompare,
    /// No usable altitude source.
    Unavailable,
}

impl AltFault {
    const fn code(self) -> u8 {
        match self {
            Self::ReferenceLost => 1,
            Self::DatumMiscompare => 2,
            Self::Unavailable => 3,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::ReferenceLost),
            2 => Some(Self::DatumMiscompare),
            3 => Some(Self::Unavailable),
            _ => None,
        }
    }

    const fn class(self) -> AlertClass {
        AlertClass::Caution
    }
}

/// Heading and navigation reference faults (from the navigation monitor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavFault {
    /// Heading reference lost; the directional datum is untrusted.
    HeadingReferenceLost,
    /// The selected course source is invalid or unidentifiable.
    CourseSourceInvalid,
    /// No usable navigation source.
    Unavailable,
}

impl NavFault {
    const fn code(self) -> u8 {
        match self {
            Self::HeadingReferenceLost => 1,
            Self::CourseSourceInvalid => 2,
            Self::Unavailable => 3,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::HeadingReferenceLost),
            2 => Some(Self::CourseSourceInvalid),
            3 => Some(Self::Unavailable),
            _ => None,
        }
    }

    const fn class(self) -> AlertClass {
        match self {
            Self::HeadingReferenceLost | Self::CourseSourceInvalid => AlertClass::Caution,
            Self::Unavailable => AlertClass::Advisory,
        }
    }
}

/// Turn and slip indication faults (from the turn/slip monitor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynFault {
    /// Turn-rate estimate invalid.
    TurnRateInvalid,
    /// Slip/skid estimate invalid.
    SlipInvalid,
    /// No usable turn/slip source.
    Unavailable,
}

impl DynFault {
    const fn code(self) -> u8 {
        match self {
            Self::TurnRateInvalid => 1,
            Self::SlipInvalid => 2,
            Self::Unavailable => 3,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::TurnRateInvalid),
            2 => Some(Self::SlipInvalid),
            3 => Some(Self::Unavailable),
            _ => None,
        }
    }

    const fn class(self) -> AlertClass {
        AlertClass::Advisory
    }
}

/// Source-miscompare faults (from the miscompare monitor). The miscompare
/// algorithm itself is out of scope; this is only its declared result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiscompareFault {
    /// Attitude sources disagree — loss of a trustworthy attitude.
    Attitude,
    /// Airspeed sources disagree.
    Airspeed,
    /// Altitude sources disagree.
    Altitude,
    /// Heading sources disagree.
    Heading,
}

impl MiscompareFault {
    const fn code(self) -> u8 {
        match self {
            Self::Attitude => 1,
            Self::Airspeed => 2,
            Self::Altitude => 3,
            Self::Heading => 4,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Attitude),
            2 => Some(Self::Airspeed),
            3 => Some(Self::Altitude),
            4 => Some(Self::Heading),
            _ => None,
        }
    }

    const fn class(self) -> AlertClass {
        match self {
            Self::Attitude => AlertClass::Warning,
            Self::Airspeed | Self::Altitude | Self::Heading => AlertClass::Caution,
        }
    }
}

/// Display and renderer health faults (DISP-01-style reason codes, from the
/// renderer-health monitor, AIR-IN-013).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayFault {
    /// The renderer stopped making progress.
    RendererStalled,
    /// Frame generation stopped advancing.
    FrameGenerationLost,
    /// The draw-command buffer failed its integrity check.
    CommandBufferCorrupt,
    /// The rendering backend was lost.
    BackendLost,
    /// A retained last-good image is suspected on the output path.
    RetainedImage,
}

impl DisplayFault {
    const fn code(self) -> u8 {
        match self {
            Self::RendererStalled => 1,
            Self::FrameGenerationLost => 2,
            Self::CommandBufferCorrupt => 3,
            Self::BackendLost => 4,
            Self::RetainedImage => 5,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::RendererStalled),
            2 => Some(Self::FrameGenerationLost),
            3 => Some(Self::CommandBufferCorrupt),
            4 => Some(Self::BackendLost),
            5 => Some(Self::RetainedImage),
            _ => None,
        }
    }

    const fn class(self) -> AlertClass {
        match self {
            Self::RendererStalled
            | Self::FrameGenerationLost
            | Self::CommandBufferCorrupt
            | Self::RetainedImage => AlertClass::Warning,
            Self::BackendLost => AlertClass::Caution,
        }
    }
}

/// Non-flight system notes: status and maintenance information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemNote {
    /// A navigation or terrain database is out of date.
    DatabaseStale,
    /// Maintenance action is required (ground-crew information).
    MaintenanceRequired,
    /// A configuration mismatch was detected.
    ConfigMismatch,
}

impl SystemNote {
    const fn code(self) -> u8 {
        match self {
            Self::DatabaseStale => 1,
            Self::MaintenanceRequired => 2,
            Self::ConfigMismatch => 3,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::DatabaseStale),
            2 => Some(Self::MaintenanceRequired),
            3 => Some(Self::ConfigMismatch),
            _ => None,
        }
    }

    const fn class(self) -> AlertClass {
        match self {
            Self::DatabaseStale | Self::ConfigMismatch => AlertClass::Status,
            Self::MaintenanceRequired => AlertClass::Maintenance,
        }
    }
}

/// A typed fault condition a producer asserts or clears. The manager reads
/// only its stable identity and fixed class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertCondition {
    /// Altitude reference/datum fault.
    Altitude(AltFault),
    /// Heading/navigation reference fault.
    Heading(NavFault),
    /// Turn/slip indication fault.
    TurnSlip(DynFault),
    /// Declared source miscompare.
    Miscompare(MiscompareFault),
    /// Display/renderer health fault.
    Display(DisplayFault),
    /// Frame-identity mismatch. Frame types are owned elsewhere
    /// (FRAME-01); this seam carries only an opaque fault code so the
    /// manager can raise the alert before those types land.
    FrameMismatch {
        /// Opaque frame-mismatch reason code.
        code: u8,
    },
    /// Non-flight status or maintenance note.
    System(SystemNote),
}

impl AlertCondition {
    /// The stable identity this condition raises.
    pub const fn id(self) -> AlertId {
        match self {
            Self::Altitude(f) => id(FAMILY_ALTITUDE, f.code()),
            Self::Heading(f) => id(FAMILY_NAV, f.code()),
            Self::TurnSlip(f) => id(FAMILY_TURN_SLIP, f.code()),
            Self::Miscompare(f) => id(FAMILY_MISCOMPARE, f.code()),
            Self::Display(f) => id(FAMILY_DISPLAY, f.code()),
            Self::FrameMismatch { code } => id(FAMILY_FRAME, code),
            Self::System(f) => id(FAMILY_SYSTEM, f.code()),
        }
    }

    /// The fixed severity class this condition carries.
    pub const fn class(self) -> AlertClass {
        match self {
            Self::Altitude(f) => f.class(),
            Self::Heading(f) => f.class(),
            Self::TurnSlip(f) => f.class(),
            Self::Miscompare(f) => f.class(),
            Self::Display(f) => f.class(),
            Self::FrameMismatch { .. } => AlertClass::Caution,
            Self::System(f) => f.class(),
        }
    }
}

/// The class an identity carries, or `None` if this build does not know the
/// identity. Every frame-mismatch code resolves to caution.
pub const fn class_of(id: AlertId) -> Option<AlertClass> {
    let family = (id.0 >> 8) as u8;
    let code = (id.0 & 0x00ff) as u8;
    match family {
        FAMILY_ALTITUDE => match AltFault::from_code(code) {
            Some(f) => Some(f.class()),
            None => None,
        },
        FAMILY_NAV => match NavFault::from_code(code) {
            Some(f) => Some(f.class()),
            None => None,
        },
        FAMILY_TURN_SLIP => match DynFault::from_code(code) {
            Some(f) => Some(f.class()),
            None => None,
        },
        FAMILY_MISCOMPARE => match MiscompareFault::from_code(code) {
            Some(f) => Some(f.class()),
            None => None,
        },
        FAMILY_DISPLAY => match DisplayFault::from_code(code) {
            Some(f) => Some(f.class()),
            None => None,
        },
        FAMILY_FRAME => Some(AlertClass::Caution),
        FAMILY_SYSTEM => match SystemNote::from_code(code) {
            Some(f) => Some(f.class()),
            None => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests;
