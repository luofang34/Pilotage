//! Builds one measured Navdata baseline archive from an `.acnav` snapshot blob.

use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use pilotage_airspace_view::{IdentifiedNavdataSnapshotV1, NavdataIdentityV1, navdata_cycle_id};
use pilotage_navdata_tiles::{NavdataTileConfig, build_mbtiles};
use sha2::{Digest, Sha256};

#[derive(serde::Serialize)]
struct Measurement {
    cycle: String,
    snapshot_id: String,
    snapshot_digest: String,
    source_format_version: u16,
    source_blob_bytes: u64,
    source_points: u64,
    source_airways: u64,
    source_runways: u64,
    source_airspaces: u64,
    output_file: String,
    output_sha256: String,
    elapsed_milliseconds: u128,
    report: pilotage_navdata_tiles::NavdataTileReport,
}

fn main() -> Result<(), Box<dyn Error>> {
    let (input, output) = arguments()?;
    let blob = fs::read(&input)?;
    let source_blob_bytes = blob.len() as u64;
    let info = aerocontext_navdata::inspect(&blob)?;
    let digest = info.sha256_hex();
    let cycle = navdata_cycle_id(&info.snapshot);
    let identity = NavdataIdentityV1 {
        cycle: cycle.clone(),
        snapshot_id: format!("{cycle}:sha256:{digest}"),
        snapshot_digest: format!("sha256:{digest}"),
    };
    let snapshot = IdentifiedNavdataSnapshotV1::try_new(identity, info.snapshot)?;
    let started = Instant::now();
    let bundle = build_mbtiles(&snapshot, NavdataTileConfig::default())?;
    let elapsed_milliseconds = started.elapsed().as_millis();
    fs::write(&output, bundle.bytes())?;
    let measurement = measurement(
        &output,
        info.format_version,
        source_blob_bytes,
        &snapshot,
        bundle.report(),
        elapsed_milliseconds,
        bundle.bytes(),
    );
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    serde_json::to_writer_pretty(&mut writer, &measurement)?;
    writer.write_all(b"\n")?;
    Ok(())
}

fn arguments() -> Result<(PathBuf, PathBuf), io::Error> {
    let mut arguments = std::env::args_os().skip(1);
    let input = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing input .acnav path"))?;
    let output = arguments.next().map(PathBuf::from).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "missing output .mbtiles path")
    })?;
    if arguments.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected input and output paths only",
        ));
    }
    Ok((input, output))
}

fn measurement(
    output: &Path,
    source_format_version: u16,
    source_blob_bytes: u64,
    snapshot: &IdentifiedNavdataSnapshotV1,
    report: &pilotage_navdata_tiles::NavdataTileReport,
    elapsed_milliseconds: u128,
    bytes: &[u8],
) -> Measurement {
    let navdata = snapshot.snapshot();
    Measurement {
        cycle: snapshot.identity().cycle.clone(),
        snapshot_id: snapshot.identity().snapshot_id.clone(),
        snapshot_digest: snapshot.identity().snapshot_digest.clone(),
        source_format_version,
        source_blob_bytes,
        source_points: navdata.points.len() as u64,
        source_airways: navdata.airways.len() as u64,
        source_runways: navdata.runways.len() as u64,
        source_airspaces: navdata.airspaces.len() as u64,
        output_file: output.file_name().map_or_else(
            || output.display().to_string(),
            |name| name.to_string_lossy().into(),
        ),
        output_sha256: hex_digest(bytes),
        elapsed_milliseconds,
        report: *report,
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
