//! Shared vertical placement arithmetic.

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainHeight {
    pub metres: f64,
    pub uses_reported_altitude_fallback: bool,
}

pub(crate) fn reported_height(
    reported_altitude_m: f64,
    terrain_elevation_m: Option<f64>,
) -> TerrainHeight {
    let terrain_elevation_m = terrain_elevation_m.filter(|value| value.is_finite());
    TerrainHeight {
        metres: reported_altitude_m - terrain_elevation_m.unwrap_or(0.0),
        uses_reported_altitude_fallback: terrain_elevation_m.is_none(),
    }
}
