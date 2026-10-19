//! The world the cameras see: fast base imagery with detail imagery over it
//! at close range, draped over Terrarium terrain under
//! a daylight sky, or a complete style the operator supplies (Sokoly's own
//! style file renders its tactical map from the vehicle's eye).

use maplibre::style::Style;

use crate::args::Config;
use crate::error::CameraError;

/// The default camera style for `config`'s imagery and terrain.
#[must_use]
pub fn camera_style_json(config: &Config) -> serde_json::Value {
    serde_json::json!({
        "version": 8,
        "projection": {"type": "mercator"},
        "sources": {
            "base-imagery": {
                "type": "raster",
                "tiles": [config.base_imagery_url],
                "tileSize": 256,
                "maxzoom": config.base_imagery_max_zoom,
            },
            "imagery": {
                "type": "raster",
                "tiles": [config.imagery_url],
                "tileSize": 256,
                "minzoom": config.imagery_min_zoom,
                "maxzoom": config.imagery_max_zoom,
            },
            "terrain": {
                "type": "raster-dem",
                "tiles": [config.dem_url],
                "tileSize": 256,
                "encoding": "terrarium",
                "maxzoom": 15,
            },
        },
        "terrain": {"source": "terrain", "exaggeration": 1.0},
        "sky": {
            "sky-color": "#78a7dc",
            "horizon-color": "#dce6ef",
            "fog-color": "#dce6ef",
            "fog-ground-blend": 0.7,
            "horizon-fog-blend": 0.6,
            "sky-horizon-blend": 0.5,
        },
        "layers": [
            {"id": "ground", "type": "background", "paint": {"background-color": "#6d7a58"}},
            {"id": "base-imagery", "type": "raster", "source": "base-imagery"},
            // Detail over the base: where a detail tile has not arrived,
            // the base shows through instead of bare ground.
            {"id": "imagery", "type": "raster", "source": "imagery",
             "minzoom": config.imagery_min_zoom},
        ],
    })
}

/// The style the cameras render: the supplied file, or imagery over terrain.
///
/// # Errors
///
/// [`CameraError::StyleRead`] or [`CameraError::StyleParse`] for a style
/// file that cannot be used.
pub fn load(config: &Config) -> Result<Style, CameraError> {
    match &config.style {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|source| CameraError::StyleRead {
                path: path.clone(),
                source,
            })?;
            serde_json::from_str(&text).map_err(|source| CameraError::StyleParse {
                origin: path.display().to_string(),
                source,
            })
        }
        None => serde_json::from_value(camera_style_json(config)).map_err(|source| {
            CameraError::StyleParse {
                origin: "the built-in camera style".to_owned(),
                source,
            }
        }),
    }
}
