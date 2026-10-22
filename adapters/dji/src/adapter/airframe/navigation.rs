//! The FC's fix as a geodetic position and a kinematics group.
//!
//! The horizontal frame is a WGS-84 tangent plane anchored at the first fix
//! of a binding. The height datum is the point where the motors last
//! started: each motor start is a new local origin identity, so the height
//! step at a takeoff is an identity change and not a silent value change.

use pilotage_adapter_api::{
    AvionicsKinematicsSample, GeodeticFixSample, MeasurementStamp, SourceIncarnation,
};
use pilotage_geo::{
    BaroSettingId, DatumRealizationId, GeodeticPosition, GeoidModelId, HorizontalDatum,
    LocalOriginId, PositionQuality, TerrainRefId, VerticalDatum, VerticalPosition,
};

use crate::flight::Mini3Fix;

/// WGS-84 semi-major axis, metres.
const SEMI_MAJOR_M: f64 = 6_378_137.0;
/// WGS-84 first eccentricity squared.
const ECCENTRICITY_SQ: f64 = 6.694_379_990_14e-3;
/// Authorization bits for position (bit 2) and velocity (bit 3).
const VALID_POSITION_VELOCITY: u32 = 0b1100;
/// Estimate quality: good.
const QUALITY_GOOD: u32 = 0;
/// Estimate quality: degraded.
const QUALITY_DEGRADED: u32 = 1;
/// The satellite signal level from which the FC holds position on its own.
const GOOD_GPS_LEVEL: u8 = 4;

/// The groups one fix adds to an avionics sample.
pub(super) struct NavigationGroups {
    /// The FC's own fix, under the estimate role.
    pub(super) geodetic: GeodeticFixSample,
    /// Position from the binding's anchor and the FC's ground velocity.
    pub(super) kinematics: AvionicsKinematicsSample,
    /// Authorization bits the FC's own satellite use supports.
    pub(super) valid_flags: u32,
    /// Estimate quality the FC's own signal level supports.
    pub(super) quality: u32,
    /// Horizontal ground speed, metres per second.
    pub(super) speed_mps: f64,
}

/// Per-binding navigation state: the horizontal anchor and the height origin.
#[derive(Default)]
pub(super) struct Navigation {
    anchor: Option<LocalFrame>,
    /// Motor starts in this binding; the height origin follows this count.
    motor_starts: u32,
    motors_running: bool,
}

impl Navigation {
    /// A new binding has no anchor and no motor history.
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Follow the FC's motor flag: a start is a new height origin.
    pub(super) fn observe_motors(&mut self, running: bool) {
        if running && !self.motors_running {
            self.motor_starts = self.motor_starts.wrapping_add(1);
        }
        self.motors_running = running;
    }

    /// Build the groups for one fix, anchoring the horizontal frame at the
    /// first fix the contract accepts.
    pub(super) fn groups(
        &mut self,
        fix: &Mini3Fix,
        stamp: MeasurementStamp,
        binding_revision: u64,
    ) -> Option<NavigationGroups> {
        let origin = local_origin(
            stamp.source_incarnation,
            binding_revision,
            self.motor_starts,
        );
        let vertical = VerticalPosition::new(
            fix.height_m,
            VerticalDatum::LocalRelative,
            GeoidModelId::UNDECLARED,
            TerrainRefId::UNDECLARED,
            BaroSettingId::UNDECLARED,
            origin,
        )
        .inspect_err(|error| tracing::warn!(%error, "DJI height refused; no fix published"))
        .ok()?;
        let position = GeodeticPosition::new(
            fix.latitude_deg,
            fix.longitude_deg,
            HorizontalDatum::Wgs84,
            DatumRealizationId::UNDECLARED,
            vertical,
        )
        .inspect_err(|error| tracing::warn!(%error, "DJI fix refused; no fix published"))
        .ok()?;
        let frame = *self
            .anchor
            .get_or_insert_with(|| LocalFrame::anchored_at(fix.latitude_deg, fix.longitude_deg));
        let [north, east] = frame.north_east(fix.latitude_deg, fix.longitude_deg);
        let (valid_flags, quality) = authorization(fix);
        Some(NavigationGroups {
            geodetic: GeodeticFixSample {
                position,
                // The OSD report states a satellite count and a signal
                // level, not a 1-sigma accuracy.
                quality: PositionQuality {
                    horizontal_mm: 0,
                    vertical_mm: 0,
                },
                stamp,
            },
            kinematics: AvionicsKinematicsSample {
                pos_ned_m: [north as f32, east as f32, -fix.height_m as f32],
                vel_ned_mps: fix.velocity_ned_mps.map(|component| component as f32),
                stamp,
            },
            valid_flags,
            quality,
            speed_mps: fix.velocity_ned_mps[0].hypot(fix.velocity_ned_mps[1]),
        })
    }
}

/// The FC authorizes its position and velocity when it fuses satellite
/// positioning itself; the signal level sets the quality.
fn authorization(fix: &Mini3Fix) -> (u32, u32) {
    match (fix.gps_used, fix.gps_level >= GOOD_GPS_LEVEL) {
        (true, true) => (VALID_POSITION_VELOCITY, QUALITY_GOOD),
        (true, false) => (VALID_POSITION_VELOCITY, QUALITY_DEGRADED),
        (false, _) => (0, QUALITY_DEGRADED),
    }
}

/// The height origin's identity: this attachment, this binding, and this
/// motor start. The low bit keeps the identity declared.
fn local_origin(
    incarnation: SourceIncarnation,
    binding_revision: u64,
    motor_starts: u32,
) -> LocalOriginId {
    let bytes = incarnation.into_bytes();
    let mut head = [0u8; 8];
    head.copy_from_slice(&bytes[..8]);
    let id = u64::from_le_bytes(head)
        ^ binding_revision.rotate_left(17)
        ^ (u64::from(motor_starts) << 32);
    LocalOriginId(id | 1)
}

/// A plane tangent to the WGS-84 ellipsoid at one anchor.
#[derive(Clone, Copy)]
struct LocalFrame {
    lat_deg: f64,
    lon_deg: f64,
    metres_per_rad_lat: f64,
    metres_per_rad_lon: f64,
}

impl LocalFrame {
    fn anchored_at(lat_deg: f64, lon_deg: f64) -> Self {
        let lat = lat_deg.to_radians();
        let s = lat.sin();
        let w = (1.0 - ECCENTRICITY_SQ * s * s).sqrt();
        Self {
            lat_deg,
            lon_deg,
            metres_per_rad_lat: SEMI_MAJOR_M * (1.0 - ECCENTRICITY_SQ) / (w * w * w),
            metres_per_rad_lon: SEMI_MAJOR_M / w * lat.cos(),
        }
    }

    /// North and east metres of a point from the anchor.
    fn north_east(&self, lat_deg: f64, lon_deg: f64) -> [f64; 2] {
        [
            (lat_deg - self.lat_deg).to_radians() * self.metres_per_rad_lat,
            (lon_deg - self.lon_deg).to_radians() * self.metres_per_rad_lon,
        ]
    }
}
