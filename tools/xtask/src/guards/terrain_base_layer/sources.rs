//! The terrain source checks: which source draws at every location,
//! which source carries the deeper bands, and that every layer that
//! reads the archive paints, stacks, and switches alike.
//!
//! The archive's bands narrow as they deepen: one covers the whole
//! Earth, the next a country, the last a valley. The source that draws
//! at every location stops at the deepest band that covers the whole
//! Earth and stretches past it. A second source carries the deeper bands
//! where they exist.

use serde_json::Value;

use super::json::{
    at, at_least, at_most, field, is_number, is_text, items, layers, max_number, or_else, raw_text,
    same,
};
use super::style::WATER;
use super::{APP, Inputs, RESOLVER, require};

/// The placeholder URL of every source that reads the terrain archive.
const TERRAIN_URL: &str = "__PILOTAGE_TERRAIN_MBTILES_URL__";

/// A number as `jq -r` writes it: an integral value without a fraction.
fn number_text(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e17 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

/// The names of the style sources that read the terrain archive, in the
/// order of the style's source map.
fn terrain_sources(style: &Value) -> Vec<&String> {
    field(style, "sources")
        .as_object()
        .map(|sources| {
            sources
                .iter()
                .filter(|(_, source)| is_text(field(source, "url"), TERRAIN_URL))
                .map(|(name, _)| name)
                .collect()
        })
        .unwrap_or_default()
}

/// The layers that read a terrain source.
fn terrain_layers<'a>(style: &'a Value, terrain: &[&String]) -> Vec<&'a Value> {
    layers(style)
        .iter()
        .filter(|layer| {
            field(layer, "source")
                .as_str()
                .is_some_and(|source| terrain.iter().any(|name| name.as_str() == source))
        })
        .collect()
}

/// The source checks, in the order of the shell guard they replace.
pub(super) fn check(inputs: &Inputs, refusals: &mut Vec<String>) {
    let style = &inputs.style;
    let bands = items(field(&inputs.plan, "bands")).unwrap_or(&[]);
    let global_zoom = max_number(
        bands
            .iter()
            .filter(|band| {
                at_most(field(band, "min_lon_deg"), -180.0)
                    && at_least(field(band, "max_lon_deg"), 180.0)
            })
            .map(|band| field(band, "max_zoom")),
    );
    let unset = Value::String("unset".to_owned());
    let base = or_else(
        at(style, &["sources", "pilotage-terrain", "maxzoom"]),
        &unset,
    );
    require(
        global_zoom.is_some_and(|zoom| is_number(base, zoom)),
        &format!(
            "the terrain source drawn at every location stops at zoom {}, but the archive covers the whole Earth only to zoom {}",
            raw_text(base),
            global_zoom.map_or_else(|| "null".to_owned(), number_text)
        ),
        refusals,
    );
    let detail = detail_source(style, &inputs.source_order, bands, global_zoom);
    check_detail(inputs, detail.as_deref(), global_zoom, bands, refusals);
    check_terrain_layers(inputs, refusals);
    require(
        !inputs
            .text(RESOLVER)
            .contains("sources[\"pilotage-terrain\"]"),
        "the style resolver substitutes one named terrain source, and the archive is read by more than one",
        refusals,
    );
}

/// The first terrain source, in file order, that starts the zoom after
/// the global band stops and reaches the deepest band.
fn detail_source(
    style: &Value,
    order: &[String],
    bands: &[Value],
    global_zoom: Option<f64>,
) -> Option<String> {
    let deepest = max_number(bands.iter().map(|band| field(band, "max_zoom")));
    // jq adds 1 to a missing ceiling as to 0.
    let start = global_zoom.unwrap_or(0.0) + 1.0;
    let zero = Value::from(0);
    let sources = field(style, "sources").as_object()?;
    order
        .iter()
        .filter_map(|name| sources.get(name).map(|source| (name, source)))
        .find(|(_, source)| {
            is_text(field(source, "url"), TERRAIN_URL)
                && is_number(or_else(field(source, "minzoom"), &zero), start)
                && match deepest {
                    Some(zoom) => is_number(field(source, "maxzoom"), zoom),
                    None => field(source, "maxzoom").is_null(),
                }
        })
        .map(|(name, _)| name.clone())
}

fn check_detail(
    inputs: &Inputs,
    detail: Option<&str>,
    global_zoom: Option<f64>,
    bands: &[Value],
    refusals: &mut Vec<String>,
) {
    let deepest = max_number(bands.iter().map(|band| field(band, "max_zoom")));
    let start = global_zoom.map_or(1.0, |zoom| zoom + 1.0);
    require(
        detail.is_some(),
        &format!(
            "no terrain source carries the archive from zoom {} to its deepest band at zoom {}, so the detail it holds is not all drawn",
            number_text(start),
            deepest.map_or_else(|| "null".to_owned(), number_text)
        ),
        refusals,
    );
    if let Some(name) = detail {
        // A 3D mesh built from the global source is coarse exactly where
        // the vehicle flies, so the optional terrain build reads the
        // source that reaches deepest.
        require(
            inputs
                .text(RESOLVER)
                .contains(&format!("\"source\": \"{name}\",")),
            &format!(
                "the 3D terrain must read {name}, the source that reaches the archive's deepest band"
            ),
            refusals,
        );
    }
}

/// Every layer that reads the archive paints like the others of its
/// kind, draws below the water, and answers the terrain switch.
fn check_terrain_layers(inputs: &Inputs, refusals: &mut Vec<String>) {
    let style = &inputs.style;
    let terrain = terrain_sources(style);
    let reading = terrain_layers(style, &terrain);
    require(
        reading.iter().all(|layer| {
            reading
                .iter()
                .filter(|other| same(field(other, "type"), field(layer, "type")))
                .all(|other| same(field(other, "paint"), field(layer, "paint")))
        }),
        "the terrain layers of one kind must paint alike whichever source they read",
        refusals,
    );
    let order = layers(style);
    let index_of = |id: &Value| order.iter().position(|layer| same(field(layer, "id"), id));
    let last = reading
        .iter()
        .filter_map(|layer| index_of(field(layer, "id")))
        .max();
    require(
        WATER.iter().all(|id| index_of(&Value::from(*id)) > last),
        "hydrography must draw over every terrain layer",
        refusals,
    );
    let app = inputs.text(APP);
    for layer in reading
        .iter()
        .filter(|layer| is_text(field(layer, "type"), "hillshade"))
    {
        let id = raw_text(field(layer, "id"));
        require(
            app.contains(&format!("\"{id}\"")),
            &format!("the terrain switch does not reach the hillshade layer {id}"),
            refusals,
        );
    }
}
