//! Each case edits one fact in a copy of the tree and proves the guard
//! refuses it for the reason the case names. The guard must also accept
//! the committed tree and an unedited copy of it.

#![allow(clippy::expect_used, clippy::panic)]

mod fixture;
mod style_cases;

use serde_json::Value;

use super::{
    APP, BUILDER_LIB, BUILDER_MANIFEST, COASTLINE_PLAN, GENERATOR, GITIGNORE, MAP_VIEW, PLAN,
    PROJECT, RESOLVER, STYLE, check,
};
use fixture::{EXAMPLE, Fixture, repo_root};

const COVERAGE: &str = "every zoom the coastline declares must cover the world";

#[test]
fn the_committed_tree_passes() {
    assert_eq!(check(&repo_root()), Vec::<String>::new());
}

#[test]
fn an_unedited_copy_passes() {
    let fixture = Fixture::new("unedited");
    assert_eq!(fixture.refusals(), Vec::<String>::new());
}

#[test]
fn a_hillshade_colour_outside_the_palette_is_refused() {
    let fixture = Fixture::new("palette");
    fixture.replace(STYLE, "#cfc9b4", "#ff0000");
    fixture.assert_refused("the terrain hillshade must use the warm neutral palette");
}

fn plan_case(tag: &str, edit: impl FnOnce(&mut Value), expected: &str) {
    let fixture = Fixture::new(tag);
    fixture.edit_json(COASTLINE_PLAN, edit);
    fixture.resync_coastline_manifest();
    fixture.assert_refused(expected);
}

#[test]
fn a_regional_coastline_band_is_refused() {
    plan_case(
        "band_regional",
        |plan| {
            plan["bands"]
                .as_array_mut()
                .expect("bands")
                .push(serde_json::json!({
                    "name": "region", "min_zoom": 8, "max_zoom": 9,
                    "min_lat_deg": 45.7, "max_lat_deg": 48.1,
                    "min_lon_deg": 5.8, "max_lon_deg": 10.6,
                }));
            plan["closest_zoom"] = Value::from(9);
        },
        COVERAGE,
    );
}

#[test]
fn a_zoom_gap_in_the_coastline_bands_is_refused() {
    plan_case(
        "band_gap",
        |plan| plan["bands"][0]["min_zoom"] = Value::from(2),
        COVERAGE,
    );
}

#[test]
fn a_closest_zoom_past_the_bands_is_refused() {
    plan_case(
        "closest_past",
        |plan| {
            let closest = plan["closest_zoom"].as_u64().expect("zoom");
            plan["closest_zoom"] = Value::from(closest + 1);
        },
        COVERAGE,
    );
}

#[test]
fn a_closest_zoom_short_of_the_bands_is_refused() {
    let fixture = Fixture::new("closest_short");
    fixture.replace(COASTLINE_PLAN, "\"closest_zoom\": 7", "\"closest_zoom\": 6");
    fixture.resync_coastline_manifest();
    fixture.assert_refused(COVERAGE);
}

#[test]
fn layer_sources_that_disagree_on_fields_are_refused() {
    plan_case(
        "field_drift",
        |plan| {
            let source = plan["sources"]
                .as_array_mut()
                .expect("sources")
                .iter_mut()
                .find(|source| source["layer"] == "rivers" && source["name"] != "rivers")
                .expect("a regional river source");
            source["select"] = Value::from("featurecla");
        },
        "sources feeding one layer must agree on fields and geometry",
    );
}

#[test]
fn a_resolver_that_is_not_an_mbtiles_url_is_refused() {
    let fixture = Fixture::new("scheme");
    fixture.replace(
        RESOLVER,
        "components.scheme = \"mbtiles\"",
        "components.scheme = \"file\"",
    );
    fixture.assert_refused("the client must resolve an absolute bundled mbtiles URL at run time");
}

#[test]
fn a_terrain_manifest_for_another_plan_is_refused() {
    let fixture = Fixture::new("terrain_manifest");
    fixture.replace(PLAN, "\"min_zoom\": 6", "\"min_zoom\": 7");
    fixture.assert_refused("the terrain manifest does not describe the committed plan");
}

#[test]
fn committed_map_archives_are_refused() {
    let fixture = Fixture::new("gitignore");
    fixture.drop_lines(
        GITIGNORE,
        "clients/apple/Resources/SituationTerrain.mbtiles",
    );
    fixture.drop_lines(
        GITIGNORE,
        "clients/apple/Resources/SituationCoastline.mbtiles",
    );
    fixture.assert_refused("map archives must stay build artifacts");
}

#[test]
fn a_map_with_no_closest_zoom_is_refused() {
    let fixture = Fixture::new("closest_zoom");
    fixture.replace(MAP_VIEW, "mapView.maximumZoomLevel = maximumZoomLevel", "");
    fixture.assert_refused("the map must apply a closest zoom");
}

#[test]
fn a_generator_that_skips_an_archive_is_refused() {
    let fixture = Fixture::new("generator");
    fixture.drop_lines(GENERATOR, "build-situation-coastline.sh");
    fixture.assert_refused("project generation must build build-situation-coastline.sh");
}

#[test]
fn an_svs_package_path_in_the_build_is_refused() {
    let fixture = Fixture::new("package_lib");
    fixture.append(BUILDER_LIB, "\nbuild_package(source);\n");
    fixture.assert_refused("the cosmetic terrain build must not read or build an SVS-02 package");
}

#[test]
fn an_svs_package_path_in_an_example_is_refused() {
    let fixture = Fixture::new("package_example");
    fixture.append(EXAMPLE, "\nbuild_package(source);\n");
    fixture.assert_refused("the cosmetic terrain build must not read or build an SVS-02 package");
}

#[test]
fn an_application_without_the_resources_is_refused() {
    let fixture = Fixture::new("project");
    fixture.drop_lines(PROJECT, "- path: Resources");
    fixture.assert_refused("the Apple target must include the terrain archive as a resource");
}

#[test]
fn an_svs_database_dependency_is_refused() {
    let fixture = Fixture::new("svs_db");
    fixture.append(BUILDER_MANIFEST, "\npilotage-svs-db = \"0.1\"\n");
    fixture.assert_refused("the cosmetic terrain crate must not depend on pilotage-svs-db");
}

#[test]
fn a_resolver_that_names_one_terrain_source_is_refused() {
    let fixture = Fixture::new("one_source");
    fixture.replace(
        RESOLVER,
        "let terrainNames = sources.compactMap",
        "let _ = sources[\"pilotage-terrain\"]; let terrainNames = sources.compactMap",
    );
    fixture.assert_refused("the style resolver substitutes one named terrain source");
}

#[test]
fn a_terrain_switch_that_misses_the_detail_hillshade_is_refused() {
    let fixture = Fixture::new("switch");
    fixture.replace(APP, ", \"pilotage-terrain-hillshade-detail\"", "");
    fixture.assert_refused(
        "the terrain switch does not reach the hillshade layer pilotage-terrain-hillshade-detail",
    );
}

#[test]
fn a_3d_terrain_from_the_global_source_is_refused() {
    let fixture = Fixture::new("mesh");
    fixture.replace(
        RESOLVER,
        "\"source\": \"pilotage-terrain-detail\",",
        "\"source\": \"pilotage-terrain\",",
    );
    fixture.assert_refused("the 3D terrain must read pilotage-terrain-detail");
}

/// Half of 4^7 is 8192. One tile short of it is a plan the build did not
/// keep.
#[test]
fn a_coastline_archive_that_covers_a_corner_is_refused() {
    let fixture = Fixture::new("archive_sparse");
    fixture.coastline_archive(8191);
    fixture.assert_refused("the coastline archive does not cover the world at its deepest zoom");
}

#[test]
fn a_coastline_archive_that_covers_the_world_passes() {
    let fixture = Fixture::new("archive_full");
    fixture.coastline_archive(8192);
    assert_eq!(fixture.refusals(), Vec::<String>::new());
}

#[test]
fn a_missing_required_file_is_refused_before_any_other_check() {
    let fixture = Fixture::new("missing");
    std::fs::remove_file(fixture.path(MAP_VIEW)).expect("remove");
    let refusals = fixture.refusals();
    assert_eq!(refusals.len(), 1, "{refusals:#?}");
    assert!(
        refusals
            .iter()
            .all(|r| r.contains("required terrain file is missing"))
    );
}
