//! The plan and manifest checks: what the archives must contain, what
//! one build produced, and the attribution the tile sources require.

use serde_json::Value;

use super::json::{
    above, at, at_least, at_most, field, file_digest, is_number, is_text, items, length,
    max_number, or_else, raw_text, same,
};
use super::{COASTLINE_PLAN, COASTLINE_PROVENANCE, Inputs, PLAN, PROVENANCE, require};

/// A band that reaches both edges of the map in longitude.
fn spans_longitude(band: &Value) -> bool {
    at_most(field(band, "min_lon_deg"), -180.0) && at_least(field(band, "max_lon_deg"), 180.0)
}

/// `closest_zoom` is the deepest band, and each zoom from 0 to it is in
/// exactly one band.
fn zooms_once_each(plan: &Value, bands: &[Value]) -> bool {
    let closest = field(plan, "closest_zoom");
    let deepest = max_number(bands.iter().map(|band| field(band, "max_zoom")));
    let Some(closest_zoom) = deepest.filter(|deepest| is_number(closest, *deepest)) else {
        return false;
    };
    // jq's `range(0; closest_zoom + 1)`: every zoom from 0 below the end.
    (0_u32..)
        .map(f64::from)
        .take_while(|zoom| *zoom < closest_zoom + 1.0)
        .all(|zoom| {
            bands
                .iter()
                .filter(|band| {
                    at_most(field(band, "min_zoom"), zoom)
                        && at_least(field(band, "max_zoom"), zoom)
                })
                .count()
                == 1
        })
}

/// Every zoom the coastline plan declares covers the whole world.
pub(super) fn check_coastline_coverage(inputs: &Inputs, refusals: &mut Vec<String>) {
    let plan = &inputs.coastline_plan;
    let covers = items(field(plan, "bands")).is_some_and(|bands| {
        !bands.is_empty()
            && bands.iter().all(|band| {
                spans_longitude(band)
                    && at_most(field(band, "min_lat_deg"), -85.0)
                    && at_least(field(band, "max_lat_deg"), 85.0)
            })
            && zooms_once_each(plan, bands)
    });
    require(
        covers,
        "every zoom the coastline declares must cover the world",
        refusals,
    );
}

/// The plan, manifest, and attribution checks.
pub(super) fn check(inputs: &Inputs, refusals: &mut Vec<String>) {
    require(
        terrain_plan_is_whole(&inputs.plan),
        "the terrain plan must cover the world and a flown region in Terrarium",
        refusals,
    );
    require(
        coastline_plan_is_verified(&inputs.coastline_plan),
        "the coastline plan must define verified data for each map zoom",
        refusals,
    );
    require(
        layer_sources_agree(&inputs.coastline_plan),
        "sources feeding one layer must agree on fields and geometry",
        refusals,
    );
    check_manifests(inputs, refusals);
    check_attribution(inputs, refusals);
}

fn terrain_plan_is_whole(plan: &Value) -> bool {
    let Some(bands) = items(field(plan, "bands")) else {
        return false;
    };
    bands.len() >= 2
        && bands.iter().filter(|band| spans_longitude(band)).count() == 1
        && bands
            .iter()
            .any(|band| at_least(field(band, "max_zoom"), 10.0))
        && is_text(field(plan, "encoding"), "terrarium")
        && is_number(field(plan, "tile_size"), 256.0)
        && length(field(plan, "attribution")).is_some_and(|n| n > 0)
}

/// The layer a plan source feeds: its `layer`, else its `name`.
fn layer_of(source: &Value) -> &Value {
    or_else(field(source, "layer"), field(source, "name"))
}

fn coastline_plan_is_verified(plan: &Value) -> bool {
    let (Some(sources), Some(bands)) = (items(field(plan, "sources")), items(field(plan, "bands")))
    else {
        return false;
    };
    let mut layers: Vec<String> = sources.iter().map(|s| raw_text(layer_of(s))).collect();
    layers.sort();
    layers.dedup();
    let source_is_verified = |source: &Value| {
        field(source, "url")
            .as_str()
            .is_some_and(|url| url.starts_with("https://naturalearth.s3.amazonaws.com/"))
            && field(source, "sha256").as_str().is_some_and(|digest| {
                digest.len() == 64
                    && digest
                        .chars()
                        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
            })
            && field(source, "shapefile")
                .as_str()
                .is_some_and(|name| name.ends_with(".shp"))
    };
    is_text(field(plan, "dataset_scale"), "1:10m")
        && zooms_once_each(plan, bands)
        && sources.iter().all(|s| layer_of(s).is_string())
        && layers == ["lakes", "land", "ocean", "rivers"]
        && sources.iter().all(source_is_verified)
        && bands
            .iter()
            .filter(|band| is_number(field(band, "min_zoom"), 0.0) && spans_longitude(band))
            .count()
            == 1
        && length(field(plan, "attribution")).is_some_and(|n| n > 0)
}

/// Several sources feeding one layer name the same fields and geometry,
/// because GDAL drops from every feature a field that a later source
/// does not name.
fn layer_sources_agree(plan: &Value) -> bool {
    let Some(sources) = items(field(plan, "sources")) else {
        return false;
    };
    let featurecla = Value::String("featurecla".to_owned());
    let multipolygon = Value::String("MULTIPOLYGON".to_owned());
    let shape = |source: &Value| {
        (
            or_else(field(source, "select"), &featurecla).clone(),
            or_else(field(source, "geometry_type"), &multipolygon).clone(),
        )
    };
    sources.iter().all(|source| {
        let first = sources
            .iter()
            .find(|other| same(layer_of(other), layer_of(source)))
            .map(shape);
        first.is_some_and(|(select, geometry)| {
            let (own_select, own_geometry) = shape(source);
            same(&own_select, &select) && same(&own_geometry, &geometry)
        })
    })
}

fn check_manifests(inputs: &Inputs, refusals: &mut Vec<String>) {
    let empty = Value::String(String::new());
    let manifest = &inputs.manifest;
    require(
        raw_text(or_else(field(manifest, "plan_sha256"), &empty))
            == file_digest(&inputs.paths.at(PLAN)),
        "the terrain manifest does not describe the committed plan",
        refusals,
    );
    require(
        above(field(manifest, "tiles_written"), 0.0)
            && same(
                field(manifest, "tiles_written"),
                field(manifest, "tiles_requested"),
            ),
        "the terrain manifest must record a complete fetch",
        refusals,
    );
    let coastline = &inputs.coastline_manifest;
    require(
        raw_text(or_else(field(coastline, "plan_sha256"), &empty))
            == file_digest(&inputs.paths.at(COASTLINE_PLAN)),
        "the coastline manifest does not describe the committed plan",
        refusals,
    );
    let plan = &inputs.coastline_plan;
    require(
        above(field(coastline, "tiles_written"), 0.0)
            && ["dataset_scale", "closest_zoom", "sources", "bands"]
                .iter()
                .all(|key| same(field(coastline, key), field(plan, key))),
        "the coastline manifest must record a complete build",
        refusals,
    );
}

fn check_attribution(inputs: &Inputs, refusals: &mut Vec<String>) {
    for (plan, source, provenance, style_message, provenance_message) in [
        (
            &inputs.plan,
            "pilotage-terrain",
            PROVENANCE,
            "the style must carry the tile source attribution",
            "the terrain provenance must carry the tile source attribution",
        ),
        (
            &inputs.coastline_plan,
            "pilotage-coastline",
            COASTLINE_PROVENANCE,
            "the style must carry the coastline attribution",
            "the coastline provenance must carry the attribution",
        ),
    ] {
        let text = raw_text(field(plan, "attribution"));
        require(
            is_text(
                at(&inputs.style, &["sources", source, "attribution"]),
                &text,
            ),
            style_message,
            refusals,
        );
        require(
            inputs.text(provenance).contains(&text),
            provenance_message,
            refusals,
        );
    }
}
