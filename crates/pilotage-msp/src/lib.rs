//! MultiWii Serial Protocol (MSP) codec for Betaflight-family flight
//! controllers.
//!
//! The crate is sans-IO (ADR-0002): [`frame`] turns bytes into frames and
//! frames into bytes for MSP v1 (XOR checksum) and MSP v2 (CRC-8 DVB-S2),
//! and [`message`] turns the payloads an adapter needs into typed values.
//! A transport (TCP to SITL, a serial port to a real board) only moves the
//! bytes. Every decoder reads the Betaflight wire layout of the message and
//! refuses a payload that is too short; a payload with extra trailing
//! bytes from a newer API version decodes, because MSP appends fields.

pub mod command;
pub mod frame;
pub mod message;

pub use frame::{DecodeError, Decoder, Direction, EncodeError, Frame, Version, crc8_dvb_s2};
pub use message::{
    Altitude, ApiVersion, ArmingBlocker, ArmingDisableFlags, Attitude, BatteryState, BoxIds,
    FailsafeConfig, FailsafeProcedure, ModeFlags, ModeRange, PayloadError, RawGps, Status,
    decode_features, decode_multiple, encode_multiple_request, encode_set_mode_range,
    encode_set_raw_rc, feature, permanent_id,
};
