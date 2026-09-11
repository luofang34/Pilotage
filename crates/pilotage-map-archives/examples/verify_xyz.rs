//! Compare installed archive tiles with the producer's XYZ output.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use pilotage_map_archives::{ArchiveError, PmTiles};

#[tokio::main]
async fn main() -> Result<(), ArchiveError> {
    tracing_subscriber::fmt().init();
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err(ArchiveError::Header {
            path: PathBuf::new(),
            reason: "usage: verify_xyz PMTILES XYZ_DIRECTORY",
        });
    }
    let path = PathBuf::from(&args[0]);
    let directory = PathBuf::from(&args[1]);
    let reader = PmTiles::open(&path).await?;
    let paths = tile_paths_blocking(&directory)?;
    if paths.is_empty() {
        return Err(ArchiveError::Header {
            path: directory,
            reason: "no XYZ tiles",
        });
    }
    let mut count = 0u64;
    for file in paths {
        let coordinate = tile_coordinate(&file, &directory)?;
        let expected = fs::read(&file).map_err(|source| ArchiveError::Io {
            path: file.clone(),
            source,
        })?;
        let actual = reader
            .tile(coordinate.0, coordinate.1, coordinate.2)
            .await?;
        if actual.as_deref() != Some(expected.as_slice()) {
            return Err(ArchiveError::Header {
                path: file,
                reason: "archive differs from XYZ bytes",
            });
        }
        count = count.wrapping_add(1);
    }
    tracing::info!(archive = %path.display(), tiles = count, "all XYZ tiles match the file-backed reader");
    Ok(())
}

fn tile_paths_blocking(directory: &Path) -> Result<Vec<PathBuf>, ArchiveError> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(|source| ArchiveError::Io {
        path: directory.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| ArchiveError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            paths.extend(tile_paths_blocking(&path)?);
        } else if path.extension().is_some_and(|value| value == "mvt") {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn tile_coordinate(path: &Path, root: &Path) -> Result<(u8, u32, u32), ArchiveError> {
    let error = || ArchiveError::Header {
        path: path.to_path_buf(),
        reason: "invalid XYZ path",
    };
    let relative = path
        .strip_prefix(root)
        .map_err(|_| error())?
        .with_extension("");
    let parts: Vec<_> = relative.iter().map(|p| p.to_string_lossy()).collect();
    if parts.len() != 3 {
        return Err(error());
    }
    Ok((
        parts[0].parse().map_err(|_| error())?,
        parts[1].parse().map_err(|_| error())?,
        parts[2].parse().map_err(|_| error())?,
    ))
}
