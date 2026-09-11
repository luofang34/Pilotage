#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use ed25519_dalek::{Signer, SigningKey};

use crate::{
    CatalogTrust, Channel, PackagePath, Product, SignedCatalog,
    fixtures::{catalog, release},
};

#[test]
fn signed_catalog_rejects_tampering_rollback_and_expiry() {
    let signing = SigningKey::from_bytes(&[7; 32]);
    let payload = serde_json::to_string(&catalog(vec![release("low-1", b"chart")])).expect("JSON");
    let mut signed = SignedCatalog {
        key_id: "publisher".to_owned(),
        signature: signing.sign(payload.as_bytes()).to_bytes().to_vec(),
        payload,
    };
    let mut trust = CatalogTrust {
        keys: BTreeMap::from([("publisher".to_owned(), signing.verifying_key().to_bytes())]),
        minimum_sequence: 7,
        now: 150,
    };
    assert!(signed.verify(&trust).is_ok());
    trust.minimum_sequence = 8;
    assert!(signed.verify(&trust).is_err());
    trust.minimum_sequence = 7;
    trust.now = 300;
    assert!(signed.verify(&trust).is_err());
    trust.now = 150;
    signed.payload.push(' ');
    assert!(signed.verify(&trust).is_err());
}

#[test]
fn current_selection_uses_exact_utc_boundary_and_same_cycle_revision() {
    let first = release("low-1", b"first");
    let mut correction = release("low-2", b"correction");
    correction.revision = 1;
    let mut next = release("low-next", b"next");
    next.validity = Some(crate::Validity {
        effective_at: 200,
        expires_at: 300,
    });
    let catalog = catalog(vec![first, correction.clone(), next.clone()]);
    let current = |utc| {
        catalog.current(
            Product::IfrLow,
            "faa",
            "test-region",
            utc,
            Channel::Development,
        )
    };
    assert!(current(99).is_none());
    assert_eq!(current(100), Some(&correction));
    assert_eq!(current(199), Some(&correction));
    assert_eq!(current(200), Some(&next));
    assert!(current(300).is_none());
    assert_eq!(
        catalog.upcoming(
            Product::IfrLow,
            "faa",
            "test-region",
            150,
            Channel::Development
        ),
        Some(&next)
    );
}

#[test]
fn dependency_order_is_exact_and_cycles_are_rejected() {
    let terrain = release("terrain", b"terrain");
    let mut chart = release("chart", b"chart");
    chart.dependencies.push(terrain.id.clone());
    let mut catalog = catalog(vec![chart.clone(), terrain.clone()]);
    let ids: Vec<_> = catalog
        .installation_order(&chart.id)
        .expect("order")
        .into_iter()
        .map(|r| &r.id)
        .collect();
    assert_eq!(ids, vec![&terrain.id, &chart.id]);
    catalog.releases[1].dependencies.push(chart.id);
    assert!(catalog.validate().is_err());
}

#[test]
fn paths_and_case_collisions_cannot_escape_or_alias_installation() {
    for path in ["../data", "/tmp/data", "a/../../b", "a\\b", "", "a//b"] {
        assert!(PackagePath::try_from(path.to_owned()).is_err(), "{path}");
    }
    let mut chart = release("chart", b"chart");
    let mut duplicate = chart.artifacts[0].clone();
    duplicate.path = PackagePath::try_from("Symbols/atlas.bin".to_owned()).expect("path");
    chart.artifacts.push(duplicate);
    assert!(chart.validate().is_err());
}
