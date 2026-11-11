//! The cases that edit the situation style.

use serde_json::Value;

use super::super::STYLE;
use super::fixture::Fixture;

fn style_case(tag: &str, edit: impl FnOnce(&mut Value), expected: &str) {
    let fixture = Fixture::new(tag);
    fixture.edit_json(STYLE, edit);
    fixture.assert_refused(expected);
}

fn layer_mut<'a>(style: &'a mut Value, id: &str) -> &'a mut Value {
    style["layers"]
        .as_array_mut()
        .expect("layers")
        .iter_mut()
        .find(|layer| layer["id"] == id)
        .expect("the layer")
}

fn position(style: &Value, id: &str) -> usize {
    style["layers"]
        .as_array()
        .expect("layers")
        .iter()
        .position(|layer| layer["id"] == id)
        .expect("the layer")
}

/// Sets the colour after the ramp's `-1` stop to a water tone.
fn water_below_sea_level(style: &mut Value) {
    let ramp = layer_mut(style, "pilotage-terrain-relief")["paint"]["color-relief-color"]
        .as_array_mut()
        .expect("ramp");
    let stop = ramp
        .iter()
        .position(|value| value.as_f64() == Some(-1.0))
        .expect("the -1 stop");
    ramp[stop + 1] = Value::from("#081d29");
}

#[test]
fn an_elevation_only_coastline_is_refused() {
    style_case(
        "elevation_only",
        |style| {
            style["sources"]
                .as_object_mut()
                .expect("sources")
                .remove("pilotage-coastline");
            let fills = [
                "pilotage-ocean-fill",
                "pilotage-land-fill",
                "pilotage-lake-fill",
            ];
            style["layers"]
                .as_array_mut()
                .expect("layers")
                .retain(|layer| !fills.iter().any(|id| layer["id"] == *id));
            layer_mut(style, "pilotage-terrain-relief")["paint"]["color-relief-opacity"] =
                Value::from(1);
            water_below_sea_level(style);
        },
        "the style must draw coastline polygons below terrain relief",
    );
}

#[test]
fn a_sea_level_threshold_in_the_ramp_is_refused() {
    style_case(
        "threshold",
        water_below_sea_level,
        "the elevation ramp must not classify water at sea level",
    );
}

#[test]
fn a_relief_that_ignores_elevation_is_refused() {
    style_case(
        "constant",
        |style| {
            layer_mut(style, "pilotage-terrain-relief")["paint"]["color-relief-color"] =
                Value::from("#808080");
        },
        "the terrain relief must interpolate colour over elevation",
    );
}

#[test]
fn a_style_with_no_source_attribution_is_refused() {
    style_case(
        "attribution",
        |style| {
            style["sources"]["pilotage-terrain"]
                .as_object_mut()
                .expect("source")
                .remove("attribution");
        },
        "the style must carry the tile source attribution",
    );
}

#[test]
fn a_style_with_no_globe_projection_is_refused() {
    style_case(
        "flat",
        |style| {
            style.as_object_mut().expect("style").remove("projection");
        },
        "the style must declare the globe projection for the web renderer",
    );
}

#[test]
fn water_under_the_relief_is_refused() {
    style_case(
        "water_under",
        |style| {
            let water = position(style, "pilotage-ocean-water");
            let relief = position(style, "pilotage-terrain-relief");
            let layers = style["layers"].as_array_mut().expect("layers");
            let layer = layers.remove(water);
            layers.insert(relief, layer);
        },
        "hydrography must draw over the relief",
    );
}

#[test]
fn an_open_water_tone_no_guard_pins_is_refused() {
    style_case(
        "water_tone",
        |style| {
            layer_mut(style, "pilotage-ocean-water")["paint"]["fill-color"] =
                Value::from("#ff00ff");
        },
        "hydrography must draw over the relief",
    );
}

#[test]
fn inland_water_that_reads_as_open_water_is_refused() {
    style_case(
        "water_same",
        |style| {
            let ocean = layer_mut(style, "pilotage-ocean-water")["paint"]["fill-color"].clone();
            layer_mut(style, "pilotage-lake-water")["paint"]["fill-color"] = ocean;
        },
        "hydrography must draw over the relief",
    );
}

#[test]
fn open_water_from_the_land_polygons_is_refused() {
    style_case(
        "water_land",
        |style| {
            layer_mut(style, "pilotage-ocean-water")["source-layer"] = Value::from("land");
        },
        "hydrography must draw over the relief",
    );
}

#[test]
fn a_drainage_that_never_thins_is_refused() {
    style_case(
        "river_flat",
        |style| {
            layer_mut(style, "pilotage-river")["paint"]["line-width"] = Value::from(1.0);
        },
        "river width must thin by the rank the data carries",
    );
}

#[test]
fn a_land_tone_no_guard_pins_is_refused() {
    style_case(
        "land_tone",
        |style| {
            layer_mut(style, "pilotage-land-fill")["paint"]["fill-color"] = Value::from("#ff00ff");
        },
        "the land polygon must remain visible below terrain relief",
    );
}

#[test]
fn a_relief_that_hides_the_polygon_beneath_it_is_refused() {
    style_case(
        "relief_opaque",
        |style| {
            layer_mut(style, "pilotage-terrain-relief")["paint"]["color-relief-opacity"] =
                Value::from(1);
        },
        "the land polygon must remain visible below terrain relief",
    );
}

#[test]
fn a_terrain_source_uncapped_past_the_world_band_is_refused() {
    style_case(
        "uncapped",
        |style| {
            style["sources"]["pilotage-terrain"]
                .as_object_mut()
                .expect("source")
                .remove("maxzoom");
        },
        "the terrain source drawn at every location stops at zoom unset",
    );
}

#[test]
fn a_style_that_reaches_no_deeper_band_is_refused() {
    style_case(
        "no_detail",
        |style| {
            style["sources"]
                .as_object_mut()
                .expect("sources")
                .remove("pilotage-terrain-detail");
            style["layers"]
                .as_array_mut()
                .expect("layers")
                .retain(|layer| !layer["id"].as_str().expect("id").ends_with("-detail"));
        },
        "no terrain source carries the archive from zoom",
    );
}

#[test]
fn a_detail_source_that_leaves_a_zoom_gap_is_refused() {
    style_case(
        "detail_gap",
        |style| style["sources"]["pilotage-terrain-detail"]["minzoom"] = Value::from(8),
        "no terrain source carries the archive from zoom",
    );
}

#[test]
fn a_detail_relief_painted_unlike_the_global_relief_is_refused() {
    style_case(
        "detail_drift",
        |style| {
            layer_mut(style, "pilotage-terrain-relief-detail")["paint"]["color-relief-opacity"] =
                Value::from(0.5);
        },
        "the terrain layers of one kind must paint alike whichever source they read",
    );
}

#[test]
fn water_under_the_detail_hillshade_is_refused() {
    style_case(
        "water_under_detail",
        |style| {
            let water = position(style, "pilotage-ocean-water");
            let layers = style["layers"].as_array_mut().expect("layers");
            let layer = layers.remove(water);
            let detail = layers
                .iter()
                .position(|layer| layer["id"] == "pilotage-terrain-hillshade-detail")
                .expect("the detail hillshade");
            layers.insert(detail, layer);
        },
        "hydrography must draw over every terrain layer",
    );
}
