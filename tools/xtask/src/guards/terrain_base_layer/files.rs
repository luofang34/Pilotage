//! The text checks: the terrain build crate, the Apple client sources,
//! the project file, the ignore list, and the archive builders.

use std::path::Path;

use super::{
    BUILDER, BUILDER_LIB, BUILDER_MANIFEST, BUILDER_TESTS, COASTLINE_FETCHER, GENERATOR, GITIGNORE,
    Inputs, MAP_VIEW, PROJECT, RESOLVER, STYLE, require,
};

/// The names that belong to the SVS-02 package path.
const PACKAGE_NAMES: [&str; 4] = [
    "CandidatePackage",
    "build_package",
    "decode_package",
    "verify_artifact",
];

/// The terrain build crate takes a source dataset, never touches an
/// SVS-02 package, and compares complete archive bytes in its tests.
pub(super) fn check_builder(inputs: &Inputs, refusals: &mut Vec<String>) {
    require(
        takes_source_dataset(&inputs.text(BUILDER_LIB)),
        "the terrain build must take SourceDataset directly",
        refusals,
    );
    require(
        !any_rust_file_names_a_package(&inputs.paths.at(BUILDER)),
        "the cosmetic terrain build must not read or build an SVS-02 package",
        refusals,
    );
    require(
        !inputs.text(BUILDER_MANIFEST).contains("pilotage-svs-db"),
        "the cosmetic terrain crate must not depend on pilotage-svs-db",
        refusals,
    );
    require(
        inputs
            .text(BUILDER_TESTS)
            .contains("assert_eq!(first.bytes(), second.bytes())"),
        "the terrain build must compare complete archive bytes",
        refusals,
    );
}

/// True when some `source: &` parameter has a type that ends in
/// `SourceDataset`, written with letters and underscores only.
pub(super) fn takes_source_dataset(text: &str) -> bool {
    text.match_indices("source: &").any(|(start, marker)| {
        let rest = text.get(start + marker.len()..).unwrap_or_default();
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphabetic() || *c == '_')
            .collect();
        name.contains("SourceDataset")
    })
}

fn any_rust_file_names_a_package(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            any_rust_file_names_a_package(&path)
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            std::fs::read_to_string(&path)
                .is_ok_and(|text| PACKAGE_NAMES.iter().any(|name| text.contains(name)))
        } else {
            false
        }
    })
}

/// The client resolves the bundled archives offline, stops at the
/// closest zoom, and keeps the archives out of version control.
pub(super) fn check_client(inputs: &Inputs, refusals: &mut Vec<String>) {
    let resolver = inputs.text(RESOLVER);
    let map_view = inputs.text(MAP_VIEW);
    require(
        resolver.contains("SituationTerrain.manifest") && resolver.contains("maximumZoomLevel"),
        "the closest zoom must be read from the terrain manifest",
        refusals,
    );
    require(
        map_view.contains("mapView.maximumZoomLevel = maximumZoomLevel"),
        "the map must apply a closest zoom",
        refusals,
    );
    let style = inputs.text(STYLE).to_ascii_lowercase();
    require(
        !(style.contains("http://") || style.contains("https://")),
        "the base style must have no network URL",
        refusals,
    );
    require(
        [
            "components.scheme = \"mbtiles\"",
            "archiveURL.path.hasPrefix(\"/\")",
            "forResource: \"SituationCoastline\"",
            "forResource: \"SituationTerrain\"",
        ]
        .iter()
        .all(|needle| resolver.contains(needle)),
        "the client must resolve an absolute bundled mbtiles URL at run time",
        refusals,
    );
    require(
        map_view.contains("MLNMapView(frame: .zero, styleJSON: styleJSON)"),
        "MapLibre must receive the resolved in-memory style",
        refusals,
    );
    require(
        inputs.text(PROJECT).lines().any(is_resources_path_line),
        "the Apple target must include the terrain archive as a resource",
        refusals,
    );
    let ignored = inputs.text(GITIGNORE);
    require(
        [
            "clients/apple/Resources/SituationTerrain.mbtiles",
            "clients/apple/Resources/SituationCoastline.mbtiles",
        ]
        .iter()
        .all(|entry| ignored.split('\n').any(|line| line == *entry)),
        "map archives must stay build artifacts",
        refusals,
    );
    let fetcher = inputs.text(COASTLINE_FETCHER);
    require(
        [
            "GDAL_NUM_THREADS=1 ogr2ogr -f MBTiles",
            "shasum -a 256",
            "CONF=$work/layers.json",
        ]
        .iter()
        .all(|needle| fetcher.contains(needle)),
        "the coastline builder must verify and pack the source polygons",
        refusals,
    );
}

/// A project line `- path: Resources`, with any surrounding white space.
fn is_resources_path_line(line: &str) -> bool {
    line.trim_start()
        .strip_prefix("- path: Resources")
        .is_some_and(|rest| rest.trim().is_empty())
}

/// Project generation builds both archives that the style needs.
pub(super) fn check_generator(inputs: &Inputs, refusals: &mut Vec<String>) {
    let generator = inputs.text(GENERATOR);
    for builder in ["build-situation-terrain.sh", "build-situation-coastline.sh"] {
        require(
            generator.contains(builder),
            &format!("project generation must build {builder}"),
            refusals,
        );
    }
}
