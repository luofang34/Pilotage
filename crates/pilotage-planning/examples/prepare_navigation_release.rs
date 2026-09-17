//! Convert local FAA source files into an immutable development release.

use aerocontext_core::{NavDataAuthority, NavDataCycle, NavDataSnapshot};
use aerocontext_navdata::ingest::{CsvInputs, PackReport, extract_entry};
use chrono::{DateTime, FixedOffset};
use pilotage_planning::{NavigationDataset, NavigationIndex};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{env, error::Error, fs, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<_> = env::args().skip(1).collect();
    let [format, input, output, release_id, effective, expiry] = arguments.as_slice() else {
        return Err("usage: prepare_navigation_release <nasr|cifp> <input> <new-directory> <release-id> <effective-RFC3339> <expiry-RFC3339>".into());
    };
    let start = DateTime::parse_from_rfc3339(effective)?;
    let end = DateTime::parse_from_rfc3339(expiry)?;
    let raw = fs::read(input)?;
    let (snapshot, report) = ingest(format, &raw, start)?;
    if snapshot.cycle.next_effective_on != end.date_naive() || start >= end {
        return Err("expiry does not match the source cycle".into());
    }
    let bytes = aerocontext_navdata::encode(&snapshot)?;
    let mut dataset = NavigationDataset::from_acnav(release_id.clone(), &bytes)?;
    dataset.source.effective_at = start.timestamp();
    dataset.source.expires_at = end.timestamp();
    let target = Path::new(output);
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::tempdir_in(parent)?;
    fs::write(temporary.path().join("navigation.acnav"), &bytes)?;
    NavigationIndex::build_blocking(&temporary.path().join("navigation.sqlite"), &dataset)?;
    let manifest = manifest(&dataset, temporary.path(), &raw)?;
    fs::write(
        temporary.path().join("release.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    fs::write(
        temporary.path().join("ingest-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    // An existing edition must receive a new release identity instead of a replacement.
    fs::create_dir(target)?;
    for entry in fs::read_dir(temporary.path())? {
        let entry = entry?;
        fs::rename(entry.path(), target.join(entry.file_name()))?;
    }
    Ok(())
}

fn ingest(
    format: &str,
    raw: &[u8],
    start: DateTime<FixedOffset>,
) -> Result<(NavDataSnapshot, PackReport), Box<dyn Error>> {
    let authority = match format {
        "nasr" => NavDataAuthority::FaaNasr,
        "cifp" => NavDataAuthority::FaaCifp,
        _ => return Err("expected nasr or cifp".into()),
    };
    let cycle = NavDataCycle::new(
        authority,
        start.date_naive(),
        28,
        format!("local-source-sha256:{:x}", Sha256::digest(raw)),
    )?;
    if format == "cifp" {
        return Ok(aerocontext_navdata::ingest::cifp::pack_cifp(raw, &cycle)?);
    }
    let apt = extract_entry(raw, "APT_BASE.csv", "NASR")?;
    let nav = extract_entry(raw, "NAV_BASE.csv", "NASR")?;
    let fix = extract_entry(raw, "FIX_BASE.csv", "NASR")?;
    let awy = extract_entry(raw, "AWY_SEG_ALT.csv", "NASR")?;
    let runway = extract_entry(raw, "APT_RWY.csv", "NASR")?;
    let ends = extract_entry(raw, "APT_RWY_END.csv", "NASR")?;
    Ok(aerocontext_navdata::ingest::pack(
        CsvInputs {
            apt_base: &apt,
            nav_base: &nav,
            fix_base: &fix,
            awy_seg: &awy,
            apt_rwy: &runway,
            apt_rwy_end: &ends,
        },
        &cycle,
    )?)
}

fn manifest(
    dataset: &NavigationDataset,
    root: &Path,
    raw: &[u8],
) -> Result<serde_json::Value, Box<dyn Error>> {
    let mut artifacts = Vec::new();
    for (path, format) in [
        ("navigation.acnav", "acnav"),
        ("navigation.sqlite", "nav_sqlite"),
    ] {
        let bytes = fs::read(root.join(path))?;
        artifacts.push(json!({"path":path, "source":path, "format":format,
            "bytes":bytes.len(), "sha256":format!("{:x}", Sha256::digest(bytes))}));
    }
    Ok(
        json!({"schema_version":1, "id":dataset.source.release_id, "product":"navdata",
        "authority":dataset.source.authority, "edition":dataset.source.edition, "revision":1,
        "source_set":format!("{:x}", Sha256::digest(raw)), "channel":"development",
        "validity":{"effective_at":dataset.source.effective_at, "expires_at":dataset.source.expires_at},
        "coverage":{"name":"Source navigation records", "bounds":[-180,-90,180,90],
            "min_zoom":0,"max_zoom":16,"complete":false,
            "exclusions":["Point search only. Procedure legs are not supported."]},
        "artifacts":artifacts,"dependencies":[],"renderer_capabilities":[],
        "attributions":["Federal Aviation Administration"],"distribution":"unspecified"}),
    )
}
