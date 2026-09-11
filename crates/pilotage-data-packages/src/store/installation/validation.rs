use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};

use rusqlite::{Connection, OpenFlags};

use crate::{
    Artifact, ArtifactFormat, PackageError, Release,
    error::{file_error, invalid},
};

pub(super) fn validate_artifact_blocking(
    path: &Path,
    artifact: &Artifact,
    release: &Release,
) -> Result<(), PackageError> {
    match artifact.format {
        ArtifactFormat::Acnav => validate_acnav_blocking(path, release),
        ArtifactFormat::Mbtiles | ArtifactFormat::NavSqlite | ArtifactFormat::ProcedureSqlite => {
            validate_sqlite_blocking(path, artifact.format)
        }
        ArtifactFormat::Pmtiles => validate_pmtiles_blocking(path, artifact.bytes),
        ArtifactFormat::MapStyle => {
            let bytes = fs::read(path).map_err(|s| file_error(path, s))?;
            let style: serde_json::Value = serde_json::from_slice(&bytes)?;
            if style["version"] != 8 || !style["layers"].is_array() || !style["sources"].is_object()
            {
                return Err(invalid("map_style", path.display()));
            }
            Ok(())
        }
        ArtifactFormat::Pdf => {
            let mut prefix = [0u8; 5];
            File::open(path)
                .and_then(|mut f| f.read_exact(&mut prefix))
                .map_err(|s| file_error(path, s))?;
            if prefix != *b"%PDF-" {
                return Err(invalid("pdf_header", path.display()));
            }
            Ok(())
        }
        ArtifactFormat::Resource => Ok(()),
    }
}

fn validate_acnav_blocking(path: &Path, release: &Release) -> Result<(), PackageError> {
    let bytes = fs::read(path).map_err(|s| file_error(path, s))?;
    let snapshot =
        aerocontext_navdata::decode(&bytes).map_err(|source| PackageError::Navigation {
            path: path.to_path_buf(),
            source,
        })?;
    if snapshot.cycle.authority.slug() != release.authority {
        return Err(invalid("navigation_authority", &release.authority));
    }
    let validity = release
        .validity
        .ok_or_else(|| invalid("navigation_validity", "missing"))?;
    for (utc, expected) in [
        (validity.effective_at, snapshot.cycle.effective_on),
        (validity.expires_at, snapshot.cycle.next_effective_on),
    ] {
        let actual = chrono::DateTime::from_timestamp(utc, 0).map(|date| date.date_naive());
        if actual != Some(expected) {
            return Err(invalid("navigation_validity", utc));
        }
    }
    Ok(())
}

fn validate_sqlite_blocking(path: &Path, format: ArtifactFormat) -> Result<(), PackageError> {
    let database_error = |source| PackageError::Database {
        path: path.to_path_buf(),
        source,
    };
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(database_error)?;
    let integrity: String = connection
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(database_error)?;
    if integrity != "ok" {
        return Err(invalid("sqlite_integrity", integrity));
    }
    let query = match format {
        ArtifactFormat::Mbtiles => {
            "SELECT zoom_level, tile_column, tile_row, tile_data FROM tiles LIMIT 0"
        }
        ArtifactFormat::NavSqlite => {
            "SELECT identifier, latitude_deg, longitude_deg FROM points LIMIT 0"
        }
        ArtifactFormat::ProcedureSqlite => {
            "SELECT airport, identifier, kind FROM procedures LIMIT 0"
        }
        _ => return Err(invalid("sqlite_format", path.display())),
    };
    connection.prepare(query).map_err(database_error)?;
    Ok(())
}

fn validate_pmtiles_blocking(path: &Path, bytes: u64) -> Result<(), PackageError> {
    let mut header = [0u8; 127];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut header))
        .map_err(|s| file_error(path, s))?;
    if &header[..7] != b"PMTiles" || header[7] != 3 {
        return Err(invalid("pmtiles_header", path.display()));
    }
    for start in [8, 24, 40, 56] {
        let read_u64 = |index| -> Result<u64, PackageError> {
            let raw = header[index..index + 8]
                .try_into()
                .map_err(|_| invalid("pmtiles_header", index))?;
            Ok(u64::from_le_bytes(raw))
        };
        let offset = read_u64(start)?;
        let length = read_u64(start + 8)?;
        if offset.checked_add(length).is_none_or(|end| end > bytes) {
            return Err(invalid("pmtiles_range", offset));
        }
    }
    Ok(())
}
