//! The terrain base layer guard: the cosmetic terrain build stays apart
//! from the SVS-02 package path, and the offline Apple map style, its
//! plans, its manifests, and its archives keep their contract.
//!
//! The terrain and coastline archives are build artifacts and are not
//! committed. The guard checks an archive only when the working tree
//! holds one. The plans and the manifests are committed, and the guard
//! always checks them: they say which tiles an archive must contain and
//! what one build produced.

mod archive;
mod files;
mod json;
mod plans;
mod sources;
mod style;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The guard name in the runner's report.
pub(crate) const NAME: &str = "terrain-base-layer";

/// The files the guard reads, relative to the repository root.
pub(super) struct Paths {
    pub(super) root: PathBuf,
}

impl Paths {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    pub(super) fn at(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}

pub(super) const BUILDER: &str = "crates/pilotage-terrain-build";
pub(super) const BUILDER_LIB: &str = "crates/pilotage-terrain-build/src/lib.rs";
pub(super) const BUILDER_TESTS: &str = "crates/pilotage-terrain-build/src/tests.rs";
pub(super) const BUILDER_MANIFEST: &str = "crates/pilotage-terrain-build/Cargo.toml";
pub(super) const STYLE: &str = "clients/apple/Resources/SituationStyle.json";
pub(super) const ARCHIVE: &str = "clients/apple/Resources/SituationTerrain.mbtiles";
pub(super) const PLAN: &str = "clients/apple/Resources/SituationTerrain.plan.json";
pub(super) const MANIFEST: &str = "clients/apple/Resources/SituationTerrain.manifest.json";
pub(super) const FETCHER: &str = "clients/apple/scripts/build-situation-terrain.sh";
pub(super) const PROVENANCE: &str = "clients/apple/Resources/SituationTerrain.provenance.md";
pub(super) const COASTLINE_ARCHIVE: &str = "clients/apple/Resources/SituationCoastline.mbtiles";
pub(super) const COASTLINE_PLAN: &str = "clients/apple/Resources/SituationCoastline.plan.json";
pub(super) const COASTLINE_MANIFEST: &str =
    "clients/apple/Resources/SituationCoastline.manifest.json";
pub(super) const COASTLINE_FETCHER: &str = "clients/apple/scripts/build-situation-coastline.sh";
pub(super) const COASTLINE_PROVENANCE: &str =
    "clients/apple/Resources/SituationCoastline.provenance.md";
pub(super) const RESOLVER: &str = "clients/apple/App/SituationStyleResource.swift";
pub(super) const APP: &str = "clients/apple/App/PilotageApp.swift";
pub(super) const MAP_VIEW: &str = "clients/apple/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/SituationMapView.swift";
pub(super) const PROJECT: &str = "clients/apple/project.yml";
pub(super) const GENERATOR: &str = "clients/apple/scripts/generate-project.sh";
pub(super) const GITIGNORE: &str = ".gitignore";

/// The files that must exist before any other check runs.
const REQUIRED: [&str; 15] = [
    BUILDER_LIB,
    BUILDER_TESTS,
    STYLE,
    PLAN,
    MANIFEST,
    FETCHER,
    PROVENANCE,
    COASTLINE_PLAN,
    COASTLINE_MANIFEST,
    COASTLINE_FETCHER,
    COASTLINE_PROVENANCE,
    RESOLVER,
    APP,
    MAP_VIEW,
    PROJECT,
];

/// The parsed inputs that several checks share.
pub(super) struct Inputs {
    pub(super) paths: Paths,
    pub(super) style: Value,
    pub(super) plan: Value,
    pub(super) manifest: Value,
    pub(super) coastline_plan: Value,
    pub(super) coastline_manifest: Value,
}

impl Inputs {
    fn read(paths: Paths) -> Self {
        let parse = |relative: &str| json_file(&paths.at(relative));
        Self {
            style: parse(STYLE),
            plan: parse(PLAN),
            manifest: parse(MANIFEST),
            coastline_plan: parse(COASTLINE_PLAN),
            coastline_manifest: parse(COASTLINE_MANIFEST),
            paths,
        }
    }

    /// The text of a file; empty when it cannot be read, so a "must
    /// contain" check refuses and a "must not contain" check accepts.
    pub(super) fn text(&self, relative: &str) -> String {
        std::fs::read_to_string(self.paths.at(relative)).unwrap_or_default()
    }
}

/// A JSON file, or `null` when it cannot be read or parsed: every check
/// that needs a value then refuses.
fn json_file(path: &Path) -> Value {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

/// Runs every check against the tree at `root` and returns each refusal
/// as one `FORBIDDEN:` line. An empty list is a pass.
pub(crate) fn check(root: &Path) -> Vec<String> {
    let paths = Paths::new(root);
    let missing: Vec<String> = REQUIRED
        .iter()
        .filter(|relative| !paths.at(relative).is_file())
        .map(|relative| {
            format!(
                "FORBIDDEN: required terrain file is missing: {}",
                paths.at(relative).display()
            )
        })
        .collect();
    if !missing.is_empty() {
        return missing;
    }
    let inputs = Inputs::read(paths);
    let mut refusals = Vec::new();
    files::check_builder(&inputs, &mut refusals);
    style::check(&inputs, &mut refusals);
    plans::check_coastline_coverage(&inputs, &mut refusals);
    archive::check_coastline_coverage(&inputs, &mut refusals);
    style::check_paint(&inputs, &mut refusals);
    files::check_client(&inputs, &mut refusals);
    plans::check(&inputs, &mut refusals);
    archive::check_digests(&inputs, &mut refusals);
    files::check_generator(&inputs, &mut refusals);
    sources::check(&inputs, &mut refusals);
    refusals
}

/// Adds the refusal `message` unless `holds`.
pub(super) fn require(holds: bool, message: &str, refusals: &mut Vec<String>) {
    if !holds {
        refusals.push(format!("FORBIDDEN: {message}"));
    }
}
