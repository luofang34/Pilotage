//! How an autopilot puts an integer parameter in PARAM_VALUE's float field.
//!
//! The field is a float on the wire. PX4 copies an integer's bytes into it
//! (MAV_PROTOCOL_CAPABILITY_PARAM_ENCODE_BYTEWISE); ArduPilot stores the
//! number itself as a float (C-cast encoding). The same frame reads as two
//! different numbers, so a reader names the encoding its autopilot uses.

use super::ParamValue;

/// An autopilot's integer-parameter encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamEncoding {
    /// The integer's bytes in the float field (PX4).
    Bytewise,
    /// The integer's value as a float (ArduPilot).
    CCast,
}

impl ParamEncoding {
    /// The number a reported value stands for.
    #[must_use]
    pub fn number(self, value: ParamValue) -> f64 {
        match (self, value) {
            (_, ParamValue::Real32(value)) => f64::from(value),
            (Self::Bytewise, ParamValue::Int32(value)) => f64::from(value),
            (Self::CCast, ParamValue::Int32(bits)) => f64::from(f32::from_bits(bits as u32)),
        }
    }

    /// The INT32 value that writes integer `value` under this encoding.
    #[must_use]
    pub fn integer(self, value: i32) -> ParamValue {
        match self {
            Self::Bytewise => ParamValue::Int32(value),
            Self::CCast => ParamValue::Int32((value as f32).to_bits() as i32),
        }
    }
}

#[cfg(test)]
mod tests;
