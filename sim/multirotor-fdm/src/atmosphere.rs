//! Static pressure from altitude (ICAO standard atmosphere, troposphere).

/// Sea-level standard pressure, Pa.
const SEA_LEVEL_PA: f64 = 101_325.0;

/// Static pressure at `alt_m` metres above mean sea level, Pa: the same
/// barometric formula Betaflight's Gazebo bridge inverts.
#[must_use]
pub fn pressure_pa(alt_m: f64) -> f64 {
    SEA_LEVEL_PA * (1.0 - 2.255_77e-5 * alt_m).powf(5.255_88)
}
