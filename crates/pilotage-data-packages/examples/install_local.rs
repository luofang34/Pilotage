//! Install one local release through the shared package store.

use std::{env, fs, path::PathBuf};

use pilotage_data_packages::{PackageError, PackageStore, Release};

fn main() -> Result<(), PackageError> {
    tracing_subscriber::fmt().init();
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err(PackageError::Invalid {
            field: "arguments",
            value: "usage: install_local RELEASE_JSON SOURCE_DIRECTORY STORE_DIRECTORY".to_owned(),
        });
    }
    let manifest = PathBuf::from(&args[0]);
    let bytes = fs::read(&manifest).map_err(|source| PackageError::Io {
        path: manifest,
        source,
    })?;
    let release: Release = serde_json::from_slice(&bytes)?;
    let source = PathBuf::from(&args[1]);
    let mut store = PackageStore::open_blocking(PathBuf::from(&args[2]))?;
    let installed = store.import_directory_blocking(&release, &source, u64::MAX)?;
    tracing::info!(release = installed.release.id.as_str(), path = %installed.directory.display(),
        artifacts = installed.release.artifacts.len(), "local package verified and installed");
    Ok(())
}
