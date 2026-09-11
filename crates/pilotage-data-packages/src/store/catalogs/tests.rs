#![allow(clippy::expect_used)]

use ed25519_dalek::{Signer, SigningKey};

use crate::{PackageId, PackageStore, SignedCatalog, fixtures::catalog};

#[test]
fn publisher_cannot_reuse_an_id_after_removing_it_from_the_catalog() {
    let root = tempfile::tempdir().expect("temporary store");
    let publisher = PackageId::try_from("faa-test".to_owned()).expect("publisher ID");
    let signer = SigningKey::from_bytes(&[17; 32]);
    let mut release = crate::fixtures::release("immutable-release", b"chart bytes");
    let mut store = PackageStore::open_blocking(root.path()).expect("open store");
    let mut accept = |sequence, releases| {
        let mut value = catalog(releases);
        value.sequence = sequence;
        let payload = serde_json::to_string(&value).expect("catalog JSON");
        let signature = signer.sign(payload.as_bytes()).to_bytes().to_vec();
        let envelope = SignedCatalog {
            key_id: "test".to_owned(),
            payload,
            signature,
        };
        store.accept_catalog_blocking(
            &publisher,
            &envelope,
            [("test".to_owned(), signer.verifying_key().to_bytes())].into(),
            150,
        )
    };
    accept(7, vec![release.clone()]).expect("publish release");
    accept(8, Vec::new()).expect("remove release from catalog");
    release.edition = "changed-edition".to_owned();
    assert!(matches!(
        accept(9, vec![release]),
        Err(crate::PackageError::IdentityConflict { .. })
    ));
    let retained = store
        .catalog_envelope_cached_blocking(&publisher)
        .expect("read catalog")
        .expect("accepted catalog");
    assert_eq!(
        serde_json::from_str::<crate::Catalog>(&retained.payload)
            .expect("decode catalog")
            .sequence,
        8
    );
}

#[test]
fn catalog_sequence_survives_restart_and_rejects_equivocation() {
    let root = tempfile::tempdir().expect("temporary store");
    let publisher = PackageId::try_from("faa-test".to_owned()).expect("publisher ID");
    let signer = SigningKey::from_bytes(&[17; 32]);
    let keys: std::collections::BTreeMap<String, [u8; 32]> =
        [("test".to_owned(), signer.verifying_key().to_bytes())].into();
    let mut current = catalog(Vec::new());
    let sign = |value: &crate::Catalog| {
        let payload = serde_json::to_string(value).expect("catalog JSON");
        let signature = signer.sign(payload.as_bytes()).to_bytes().to_vec();
        SignedCatalog {
            key_id: "test".to_owned(),
            payload,
            signature,
        }
    };
    {
        let mut store = PackageStore::open_blocking(root.path()).expect("open store");
        store
            .accept_catalog_blocking(&publisher, &sign(&current), keys.clone(), 150)
            .expect("accept catalog");
    }
    let mut store = PackageStore::open_blocking(root.path()).expect("reopen store");
    current.sequence = 6;
    assert!(
        store
            .accept_catalog_blocking(&publisher, &sign(&current), keys.clone(), 150)
            .is_err()
    );
    current.sequence = 7;
    current.expires_at = 400;
    assert!(
        store
            .accept_catalog_blocking(&publisher, &sign(&current), keys.clone(), 150)
            .is_err()
    );
    current.sequence = 8;
    store
        .accept_catalog_blocking(&publisher, &sign(&current), keys.clone(), 150)
        .expect("accept newer catalog");
    assert!(
        store
            .accept_catalog_blocking(&publisher, &sign(&current), keys, 400)
            .is_err()
    );
    assert_eq!(
        store
            .catalog_envelope_cached_blocking(&publisher)
            .expect("read cache")
            .expect("retained envelope")
            .payload,
        sign(&current).payload
    );
}
