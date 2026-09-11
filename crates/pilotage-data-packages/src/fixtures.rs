#![allow(clippy::expect_used)]

use crate::{
    Artifact, ArtifactFormat, Catalog, Channel, ContentDigest, Coverage, Distribution, PackageId,
    PackagePath, Product, Release, SelectionPolicy, Validity,
};

pub(crate) fn release(id: &str, bytes: &[u8]) -> Release {
    Release {
        schema_version: 1,
        id: PackageId::try_from(id.to_owned()).expect("release ID"),
        product: Product::IfrLow,
        authority: "faa".to_owned(),
        revision: 0,
        edition: "test-edition".to_owned(),
        source_set: ContentDigest::of_bytes(b"FAA source fixture"),
        channel: Channel::Development,
        validity: Some(Validity {
            effective_at: 100,
            expires_at: 200,
        }),
        coverage: Coverage {
            name: "test-region".to_owned(),
            bounds: [-124.0, 37.0, -120.0, 40.0],
            min_zoom: 0,
            max_zoom: 14,
            complete: false,
            exclusions: vec!["Bounded test fixture".to_owned()],
        },
        artifacts: vec![Artifact {
            path: PackagePath::try_from("symbols/atlas.bin".to_owned()).expect("artifact path"),
            source: "symbols/atlas.bin".to_owned(),
            format: ArtifactFormat::Resource,
            bytes: bytes.len() as u64,
            sha256: ContentDigest::of_bytes(bytes),
        }],
        dependencies: Vec::new(),
        renderer_capabilities: Vec::new(),
        attributions: vec!["Test data".to_owned()],
        distribution: Distribution::Permitted,
    }
}

pub(crate) fn catalog(releases: Vec<Release>) -> Catalog {
    Catalog {
        schema_version: 1,
        sequence: 7,
        generated_at: 100,
        expires_at: 300,
        releases,
    }
}

pub(crate) fn policy() -> SelectionPolicy {
    SelectionPolicy {
        now: 150,
        channel: Channel::Development,
        renderer_capabilities: Default::default(),
        allow_outside_validity: false,
    }
}
