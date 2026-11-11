//! GPS_GLOBAL_ORIGIN: where on the ellipsoid the flight controller's local
//! frame has its horizontal origin.
//!
//! Only the latitude and longitude are read. The altitude field is a height
//! above mean sea level with no named geoid, so it cannot be compared with
//! an ellipsoidal height; the height of the local origin comes from the
//! host that set it, never from this message.

use super::FcMessage;
use super::decoding::u32_at;

/// GPS_GLOBAL_ORIGIN message id.
pub const GPS_GLOBAL_ORIGIN_ID: u32 = 49;

/// Wire order: latitude i32 @0, longitude i32 @4, altitude i32 @8 (not
/// read), then the version-2 extension time_usec u64 @12 (not read: its
/// clock is not named). MAVLink 2 trims trailing zero bytes, so the
/// zero-extending accessor reads a trimmed longitude as the zero it was.
pub(super) fn decode(payload: &[u8]) -> FcMessage {
    FcMessage::GlobalOrigin {
        lat_lon: [u32_at(payload, 0) as i32, u32_at(payload, 4) as i32],
    }
}
