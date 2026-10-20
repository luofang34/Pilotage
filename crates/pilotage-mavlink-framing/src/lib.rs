//! MAVLink 2 frame encoding and checksums without transport dependencies.

#![no_std]

mod checksum;
mod frame;

pub use checksum::compute_crc;
pub use frame::{MAGIC_V2, encode_frame_v2_from};
