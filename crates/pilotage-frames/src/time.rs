//! Clock-domain, time-scale, and epoch identity for frame transforms.

/// Which physical clock produced a reading (ADR-0009's domain
/// discipline applied to frames). Readings from different clocks are
/// never comparable without an explicit correlation step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ClockDomain {
    /// The vehicle's own boot-relative clock.
    VehicleBoot = 0,
    /// The simulation host's clock.
    Simulation = 1,
    /// A GNSS receiver's clock.
    Gnss = 2,
    /// A ground-segment clock.
    Ground = 3,
}

/// The time scale a reading is expressed in. Same clock, different
/// scale is still incomparable (leap-second handling differs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TimeScale {
    /// Monotonic since an endpoint-local origin.
    Monotonic = 0,
    /// GPS time (no leap seconds since the GPS epoch).
    Gps = 1,
    /// International Atomic Time.
    Tai = 2,
    /// Coordinated Universal Time (leap seconds apply).
    Utc = 3,
}

/// The instant a transform or state is valid at, with the clock and
/// scale that make the reading meaningful. Rotating-frame transforms
/// (ECI↔ECEF, LVLH) are only correct at their epoch; composition
/// therefore demands exact epoch identity, and any staleness budget is
/// a consumer policy, never an implicit tolerance here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Epoch {
    /// Producing clock.
    pub clock: ClockDomain,
    /// Expression scale.
    pub scale: TimeScale,
    /// Nanoseconds on that clock/scale.
    pub nanos: u64,
}
