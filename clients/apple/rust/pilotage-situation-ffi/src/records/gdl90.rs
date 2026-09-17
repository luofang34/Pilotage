//! GDL 90 appliance records that cross the Apple FFI boundary.

use super::RadioRecordBatch;

/// North reference for one appliance heading.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum Gdl90HeadingReferenceValue {
    /// Geographic north.
    TrueNorth,
    /// Magnetic north.
    MagneticNorth,
}

/// Current ownship values received from a GDL 90 appliance.
#[derive(Clone, Copy, Debug, Default, PartialEq, uniffi::Record)]
pub struct Gdl90NavigationSnapshot {
    /// Right-wing-down roll in degrees.
    pub roll_degrees: Option<f64>,
    /// Nose-up pitch in degrees.
    pub pitch_degrees: Option<f64>,
    /// Flight-valid heading in degrees.
    pub heading_degrees: Option<f64>,
    /// North reference for `heading_degrees`.
    pub heading_reference: Option<Gdl90HeadingReferenceValue>,
    /// Pressure altitude referenced to 29.92 inHg, in feet.
    pub pressure_altitude_feet: Option<f64>,
    /// Pressure-altitude vertical speed in feet per minute.
    pub vertical_speed_feet_per_minute: Option<f64>,
    /// Ownship latitude in degrees.
    pub latitude_degrees: Option<f64>,
    /// Ownship longitude in degrees.
    pub longitude_degrees: Option<f64>,
    /// Ownship course over the ground relative to true north.
    pub ground_track_degrees_true: Option<f64>,
    /// Time of the latest attitude message.
    pub attitude_monotonic_micros: u64,
    /// Time of the latest pressure-altitude message.
    pub baro_monotonic_micros: u64,
    /// Time of the latest ownship report.
    pub ownship_monotonic_micros: u64,
}

/// Result of one GDL 90 appliance transfer.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct Gdl90IngestBatch {
    /// Domain records produced by traffic reports.
    pub records: RadioRecordBatch,
    /// Current appliance navigation values.
    pub navigation: Option<Gdl90NavigationSnapshot>,
    /// Bytes accepted from this connection.
    pub bytes_consumed: u64,
    /// Frames that passed CRC validation.
    pub valid_frames: u64,
    /// Frames that failed CRC validation.
    pub crc_errors: u64,
    /// Frames that failed another framing or message check.
    pub invalid_frames: u64,
    /// Traffic reports accepted by Surveillance.
    pub traffic_reports: u64,
    /// Uplink Data messages held for the deferred UAT scope.
    pub deferred_uplink_messages: u64,
    /// Valid message types that this adapter does not consume.
    pub unsupported_messages: u64,
}
