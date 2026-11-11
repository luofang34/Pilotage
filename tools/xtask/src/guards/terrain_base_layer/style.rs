//! The style checks: the Terrarium hillshade contract, the coastline
//! polygons, the water layers, the hillshade palette, and the relief ramp.

use serde_json::Value;

use super::json::{
    at, count_layers, field, is_number, is_text, items, layer_index, layers_with_id, number,
};
use super::{Inputs, require};

/// The water layers that draw over the relief and the hillshade.
pub(super) const WATER: [&str; 3] = [
    "pilotage-ocean-water",
    "pilotage-lake-water",
    "pilotage-river",
];

/// One coastline layer the style must hold exactly once.
struct Polygon {
    id: &'static str,
    kind: &'static str,
    source_layer: &'static str,
    paint: &'static [(&'static str, Pin)],
}

/// A pinned paint value.
enum Pin {
    Text(&'static str),
    Number(f64),
}

fn holds(layer: &Value, polygon: &Polygon) -> bool {
    is_text(field(layer, "id"), polygon.id)
        && is_text(field(layer, "type"), polygon.kind)
        && is_text(field(layer, "source"), "pilotage-coastline")
        && is_text(field(layer, "source-layer"), polygon.source_layer)
        && polygon.paint.iter().all(|(key, pin)| {
            let value = at(layer, &["paint", key]);
            match pin {
                Pin::Text(text) => is_text(value, text),
                Pin::Number(n) => is_number(value, *n),
            }
        })
}

fn exactly_once(style: &Value, polygon: &Polygon) -> bool {
    count_layers(style, |layer| holds(layer, polygon)) == 1
}

/// The structural style checks.
pub(super) fn check(inputs: &Inputs, refusals: &mut Vec<String>) {
    let style = &inputs.style;
    require(
        terrarium_contract(style),
        "the situation style must use the Terrarium hillshade contract",
        refusals,
    );
    require(
        coastline_below_relief(style),
        "the style must draw coastline polygons below terrain relief",
        refusals,
    );
    require(
        hydrography_over_relief(style),
        "hydrography must draw over the relief",
        refusals,
    );
    require(
        river_thins_by_rank(style),
        "river width must thin by the rank the data carries",
        refusals,
    );
    require(
        warm_hillshade(style),
        "the terrain hillshade must use the warm neutral palette",
        refusals,
    );
    check_relief_ramp(style, refusals);
    require(
        is_text(at(style, &["projection", "type"]), "globe"),
        "the style must declare the globe projection for the web renderer",
        refusals,
    );
}

fn terrarium_contract(style: &Value) -> bool {
    let source = at(style, &["sources", "pilotage-terrain"]);
    let terrain_layer = |id: &str, kind: &str| {
        count_layers(style, |layer| {
            is_text(field(layer, "id"), id)
                && is_text(field(layer, "type"), kind)
                && is_text(field(layer, "source"), "pilotage-terrain")
        }) == 1
    };
    is_text(field(source, "type"), "raster-dem")
        && is_text(field(source, "encoding"), "terrarium")
        && is_number(field(source, "tileSize"), 256.0)
        && is_text(field(source, "url"), "__PILOTAGE_TERRAIN_MBTILES_URL__")
        && terrain_layer("pilotage-terrain-hillshade", "hillshade")
        && terrain_layer("pilotage-terrain-relief", "color-relief")
        && style.get("terrain").is_none()
}

fn coastline_below_relief(style: &Value) -> bool {
    let source = at(style, &["sources", "pilotage-coastline"]);
    let fill = |id, source_layer| Polygon {
        id,
        kind: "fill",
        source_layer,
        paint: &[],
    };
    let fills = [
        fill("pilotage-ocean-fill", "ocean"),
        fill("pilotage-land-fill", "land"),
        fill("pilotage-lake-fill", "lakes"),
    ];
    let relief = layer_index(style, "pilotage-terrain-relief");
    is_text(field(source, "type"), "vector")
        && is_text(field(source, "url"), "__PILOTAGE_COASTLINE_MBTILES_URL__")
        && fills.iter().all(|polygon| exactly_once(style, polygon))
        && fills
            .iter()
            .all(|polygon| layer_index(style, polygon.id) < relief)
}

/// The three water layers exist with their pinned tones, and each draws
/// after `pilotage-terrain-hillshade`.
pub(super) fn hydrography_over_relief(style: &Value) -> bool {
    let water = [
        Polygon {
            id: "pilotage-ocean-water",
            kind: "fill",
            source_layer: "ocean",
            paint: &[
                ("fill-color", Pin::Text("#e1eef5")),
                ("fill-opacity", Pin::Number(1.0)),
            ],
        },
        Polygon {
            id: "pilotage-lake-water",
            kind: "fill",
            source_layer: "lakes",
            paint: &[
                ("fill-color", Pin::Text("#93cde5")),
                ("fill-opacity", Pin::Number(1.0)),
            ],
        },
        Polygon {
            id: "pilotage-river",
            kind: "line",
            source_layer: "rivers",
            paint: &[("line-color", Pin::Text("#93cde5"))],
        },
    ];
    let hillshade = layer_index(style, "pilotage-terrain-hillshade");
    water.iter().all(|polygon| exactly_once(style, polygon))
        && WATER.iter().all(|id| {
            let index = layer_index(style, id);
            index.is_some() && index > hillshade
        })
}

fn river_thins_by_rank(style: &Value) -> bool {
    layers_with_id(style, "pilotage-river")
        .any(|layer| contains_text(at(layer, &["paint", "line-width"]), "scalerank"))
}

fn contains_text(value: &Value, text: &str) -> bool {
    match value {
        Value::String(found) => found == text,
        Value::Array(list) => list.iter().any(|item| contains_text(item, text)),
        Value::Object(map) => map.values().any(|item| contains_text(item, text)),
        _ => false,
    }
}

/// jq writes one answer for each hillshade layer and `-e` reads the
/// last; a style without the layer gives no answer and fails.
fn warm_hillshade(style: &Value) -> bool {
    layers_with_id(style, "pilotage-terrain-hillshade")
        .last()
        .is_some_and(|layer| {
            let paint = field(layer, "paint");
            is_text(field(paint, "hillshade-shadow-color"), "#241f16")
                && is_text(field(paint, "hillshade-highlight-color"), "#cfc9b4")
                && is_text(field(paint, "hillshade-accent-color"), "#3a3226")
        })
}

fn check_relief_ramp(style: &Value, refusals: &mut Vec<String>) {
    let ramp = layers_with_id(style, "pilotage-terrain-relief")
        .last()
        .map(|layer| at(layer, &["paint", "color-relief-color"]));
    let list = ramp.and_then(items);
    require(
        list.is_some_and(|list| {
            list.first().is_some_and(|v| is_text(v, "interpolate"))
                && list.get(2).is_some_and(|v| {
                    v.as_array().is_some_and(|a| {
                        a.len() == 1 && a.first().is_some_and(|e| is_text(e, "elevation"))
                    })
                })
                && list.len() >= 12
        }),
        "the terrain relief must interpolate colour over elevation",
        refusals,
    );
    // jq orders `null` below every number, so a missing ramp value or a
    // ramp without a fourth element passes this comparison.
    let fourth = match ramp {
        Some(Value::Null) => Some(true),
        Some(Value::Array(list)) => Some(list.get(3).is_none_or(below_zero)),
        _ => None,
    };
    require(
        fourth == Some(true),
        "the terrain relief must reach below sea level",
        refusals,
    );
    let sea = list.and_then(|list| colour_at(list, 0.0));
    let below = list.and_then(|list| colour_at(list, -1.0));
    require(
        sea.is_some() && sea == below,
        "the elevation ramp must not classify water at sea level",
        refusals,
    );
}

/// jq's `<` against 0 for a ramp element: `null` and `false` sort below
/// numbers, every other kind above them.
fn below_zero(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(_) => true,
        Value::Number(_) => number(value).is_some_and(|n| n < 0.0),
        Value::String(_) | Value::Array(_) | Value::Object(_) => false,
    }
}

/// The colour of the first stop at `elevation`, as `jq -r` prints it,
/// or `None` when there is no such stop or its colour is empty.
fn colour_at(list: &[Value], elevation: f64) -> Option<String> {
    let colour = (3..list.len())
        .step_by(2)
        .filter(|i| list.get(*i).is_some_and(|stop| is_number(stop, elevation)))
        .map(|i| list.get(i + 1).unwrap_or(&Value::Null))
        .next()?;
    let text = match colour {
        Value::Null | Value::Bool(false) => String::new(),
        other => super::json::raw_text(other),
    };
    (!text.is_empty()).then_some(text)
}

/// The land polygon stays visible through the relief, which draws at
/// part opacity; the water polygons under the relief keep their tone.
pub(super) fn check_paint(inputs: &Inputs, refusals: &mut Vec<String>) {
    let style = &inputs.style;
    let relief_part_opaque = count_layers(style, |layer| {
        is_text(field(layer, "id"), "pilotage-terrain-relief")
            && number(at(layer, &["paint", "color-relief-opacity"]))
                .is_some_and(|opacity| opacity > 0.0 && opacity < 1.0)
    }) == 1;
    require(
        opaque_fill(style, "pilotage-ocean-fill", "#061927")
            && opaque_fill(style, "pilotage-land-fill", "#dfe7c4")
            && opaque_fill(style, "pilotage-lake-fill", "#061927")
            && relief_part_opaque,
        "the land polygon must remain visible below terrain relief",
        refusals,
    );
}

/// Exactly one layer `id` fills fully opaque in `colour`.
fn opaque_fill(style: &Value, id: &str, colour: &str) -> bool {
    count_layers(style, |layer| {
        is_text(field(layer, "id"), id)
            && is_text(at(layer, &["paint", "fill-color"]), colour)
            && is_number(at(layer, &["paint", "fill-opacity"]), 1.0)
    }) == 1
}
