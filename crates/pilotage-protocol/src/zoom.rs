//! Camera zoom commands with explicit units.

use crate::{ConvertError, wire};

/// The unit of an absolute zoom target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZoomUnit {
    /// Magnification ratio.
    Ratio,
    /// Calibrated lens travel from zero to one.
    TravelFraction,
    /// Focal length in millimeters.
    FocalLengthMm,
    /// Horizontal field of view in radians.
    HorizontalFovRadians,
}

/// A camera command. Absolute targets use millionths of the named unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZoomCommand {
    /// Select an absolute zoom value without changing units.
    Absolute {
        /// Unit understood by the camera.
        unit: ZoomUnit,
        /// Value in millionths of the unit.
        value_micros: i64,
    },
    /// Move by signed device-defined detents.
    Step(i16),
    /// Direction and normalized speed in millionths, from -1000000 to 1000000.
    Continuous(i32),
}

impl ZoomUnit {
    /// Convert the unit to its wire representation.
    #[must_use]
    pub const fn to_wire(self) -> wire::ZoomUnit {
        match self {
            Self::Ratio => wire::ZoomUnit::Ratio,
            Self::TravelFraction => wire::ZoomUnit::TravelFraction,
            Self::FocalLengthMm => wire::ZoomUnit::FocalLengthMm,
            Self::HorizontalFovRadians => wire::ZoomUnit::HorizontalFovRadians,
        }
    }
}

impl From<ZoomCommand> for wire::ZoomCommand {
    fn from(command: ZoomCommand) -> Self {
        use wire::zoom_command::Operation;
        let operation = match command {
            ZoomCommand::Absolute { unit, value_micros } => {
                Operation::Absolute(wire::ZoomAbsoluteTarget {
                    unit: unit.to_wire() as i32,
                    value_micros,
                })
            }
            ZoomCommand::Step(steps) => Operation::Step(i32::from(steps)),
            ZoomCommand::Continuous(rate) => Operation::ContinuousMillionths(rate),
        };
        Self {
            operation: Some(operation),
        }
    }
}

impl TryFrom<wire::ZoomCommand> for ZoomCommand {
    type Error = ConvertError;

    fn try_from(command: wire::ZoomCommand) -> Result<Self, Self::Error> {
        use wire::zoom_command::Operation;
        let invalid = || ConvertError::MissingField {
            message: "pilotage.v1.ZoomCommand",
            field: "valid operation",
        };
        match command.operation.ok_or_else(invalid)? {
            Operation::Step(value) => i16::try_from(value).map(Self::Step).map_err(|_| invalid()),
            Operation::ContinuousMillionths(value) if (-1_000_000..=1_000_000).contains(&value) => {
                Ok(Self::Continuous(value))
            }
            Operation::ContinuousMillionths(_) => Err(invalid()),
            Operation::Absolute(target) => {
                let unit = match wire::ZoomUnit::try_from(target.unit).ok() {
                    Some(wire::ZoomUnit::Ratio) => ZoomUnit::Ratio,
                    Some(wire::ZoomUnit::TravelFraction) => ZoomUnit::TravelFraction,
                    Some(wire::ZoomUnit::FocalLengthMm) => ZoomUnit::FocalLengthMm,
                    Some(wire::ZoomUnit::HorizontalFovRadians) => ZoomUnit::HorizontalFovRadians,
                    _ => return Err(invalid()),
                };
                let valid = match unit {
                    ZoomUnit::Ratio | ZoomUnit::FocalLengthMm => target.value_micros > 0,
                    ZoomUnit::TravelFraction => (0..=1_000_000).contains(&target.value_micros),
                    ZoomUnit::HorizontalFovRadians => (1..3_141_593).contains(&target.value_micros),
                };
                if !valid {
                    return Err(invalid());
                }
                Ok(Self::Absolute {
                    unit,
                    value_micros: target.value_micros,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests;
