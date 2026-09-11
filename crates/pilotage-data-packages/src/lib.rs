//! Verify, install, and select immutable aviation data packages.

mod catalog;
mod error;
mod identity;
mod legacy;
mod manifest;
mod store;

pub use catalog::{Catalog, CatalogTrust, SignedCatalog};
pub use error::PackageError;
pub use identity::{ContentDigest, PackageId, PackagePath};
pub use legacy::import_acnav_catalog;
pub use manifest::{
    Artifact, ArtifactFormat, Channel, Coverage, Distribution, Product, Release, Validity,
};
pub use store::{
    Download, InstallPlan, InstalledRelease, PackageStore, Selection, SelectionPolicy,
};

#[cfg(test)]
mod fixtures;
