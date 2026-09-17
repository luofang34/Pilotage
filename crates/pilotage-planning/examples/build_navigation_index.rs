//! Build a query index from an ACNAV snapshot or source-neutral JSON.

use pilotage_planning::{NavigationDataset, NavigationIndex};
use std::{env, error::Error, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<_> = env::args().skip(1).collect();
    let [format, input, output, release_id] = arguments.as_slice() else {
        return Err(
            "usage: build_navigation_index <acnav|json> <input> <output> <release-id>".into(),
        );
    };
    let bytes = fs::read(input)?;
    let dataset = match format.as_str() {
        "acnav" => NavigationDataset::from_acnav(release_id.clone(), &bytes)?,
        "json" => {
            let dataset: NavigationDataset = serde_json::from_slice(&bytes)?;
            if dataset.source.release_id != *release_id {
                return Err("release identity mismatch".into());
            }
            dataset
        }
        _ => return Err("expected acnav or json".into()),
    };
    NavigationIndex::build_blocking(&PathBuf::from(output), &dataset)?;
    Ok(())
}
