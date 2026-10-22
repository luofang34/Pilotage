//! The navigation fields of the Mini 3 OSD report: the FC's own fix and the
//! satellite evidence behind it.
//!
//! The OSD report carries the fix before the body angles. The FC measures
//! the height from the point where the motors last started. A fix is the
//! FC's own solution, not an independent measurement.

use core::f64::consts::{FRAC_PI_2, PI};

use super::report::{Mini3FlightReport, osd_payload};
use crate::DumlFrame;

/// Decimetre fields become metres.
const DECIMETRES_PER_METRE: f64 = 10.0;
/// Controller-state bit: the FC uses satellite positioning as its horizontal
/// velocity sensor.
const GPS_USED: u32 = 0x8000;
/// Controller-state bits: the satellite signal level, 0 to 5.
const GPS_LEVEL_SHIFT: u32 = 18;
const GPS_LEVEL_MASK: u32 = 0x0f;

/// The raw navigation fields of one Mini 3 OSD report.
///
/// The values are the FC's own words. A caller must apply [`Self::fix`] to
/// get a position that passed the range checks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mini3PositionReport {
    /// DUML sequence of the source report.
    pub sequence: u16,
    /// Longitude, radians, positive east.
    pub longitude_rad: f64,
    /// Latitude, radians, positive north.
    pub latitude_rad: f64,
    /// Height above the point where the motors last started, decimetres.
    pub height_dm: i16,
    /// Ground velocity north, east, down, decimetres per second.
    pub velocity_ned_dmps: [i16; 3],
    /// Whether the FC fuses satellite positioning into its own estimate.
    pub gps_used: bool,
    /// The satellite signal level the FC reports, 0 (no solution) to 5.
    pub gps_level: u8,
    /// The number of satellites in the solution.
    pub satellites: u8,
}

impl Mini3PositionReport {
    /// Read the navigation fields of the fixed Mini 3 OSD layout.
    ///
    /// The report identifies the model on the observed relay route; the
    /// same checks as [`Mini3FlightReport::from_push`] apply.
    #[must_use]
    pub fn from_push(frame: &DumlFrame) -> Option<Self> {
        Mini3FlightReport::from_push(frame)?;
        let payload = osd_payload(frame)?;
        let flags = u32::from_le_bytes([payload[32], payload[33], payload[34], payload[35]]);
        let i16_at = |offset: usize| i16::from_le_bytes([payload[offset], payload[offset + 1]]);
        Some(Self {
            sequence: frame.sequence(),
            longitude_rad: f64::from_le_bytes(<[u8; 8]>::try_from(&payload[0..8]).ok()?),
            latitude_rad: f64::from_le_bytes(<[u8; 8]>::try_from(&payload[8..16]).ok()?),
            height_dm: i16_at(16),
            velocity_ned_dmps: [i16_at(18), i16_at(20), i16_at(22)],
            gps_used: flags & GPS_USED != 0,
            gps_level: ((flags >> GPS_LEVEL_SHIFT) & GPS_LEVEL_MASK) as u8,
            satellites: payload[36],
        })
    }

    /// The fix in degrees and metres.
    ///
    /// Returns `None` when the FC reports no satellite solution, when a
    /// coordinate is not finite or outside the Earth, or when both
    /// coordinates are zero, which is the FC's value before a first fix.
    #[must_use]
    pub fn fix(&self) -> Option<Mini3Fix> {
        if self.gps_level == 0 {
            return None;
        }
        let (lat, lon) = (self.latitude_rad, self.longitude_rad);
        if !lat.is_finite() || !lon.is_finite() || lat.abs() > FRAC_PI_2 || lon.abs() > PI {
            return None;
        }
        if lat == 0.0 && lon == 0.0 {
            return None;
        }
        Some(Mini3Fix {
            latitude_deg: lat.to_degrees(),
            longitude_deg: lon.to_degrees(),
            height_m: f64::from(self.height_dm) / DECIMETRES_PER_METRE,
            velocity_ned_mps: self
                .velocity_ned_dmps
                .map(|component| f64::from(component) / DECIMETRES_PER_METRE),
            gps_used: self.gps_used,
            gps_level: self.gps_level,
            satellites: self.satellites,
        })
    }
}

/// A Mini 3 fix that passed the range checks, in degrees and metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mini3Fix {
    /// Latitude, degrees, positive north.
    pub latitude_deg: f64,
    /// Longitude, degrees, positive east.
    pub longitude_deg: f64,
    /// Height above the point where the motors last started, metres.
    pub height_m: f64,
    /// Ground velocity north, east, down, metres per second.
    pub velocity_ned_mps: [f64; 3],
    /// Whether the FC fuses satellite positioning into its own estimate.
    pub gps_used: bool,
    /// The satellite signal level the FC reports, 1 to 5.
    pub gps_level: u8,
    /// The number of satellites in the solution.
    pub satellites: u8,
}
