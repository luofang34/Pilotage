#![cfg(test)]

use super::{build_with_store, hex_encode};

#[test]
fn hex_encode_is_lowercase_no_separators() {
    assert_eq!(hex_encode(&[0xAB, 0x01, 0xFF]), "ab01ff");
}

#[test]
fn dev_identity_builds_and_hashes() {
    runtime().block_on(async {
        // No store: ephemeral, touching nothing on disk.
        let dev = build_with_store(None)
            .await
            .expect("self-signed identity builds");
        assert_eq!(dev.cert_hash_hex.len(), 64);
        assert!(dev.cert_hash_hex.chars().all(|c| c.is_ascii_hexdigit()));
    });
}

#[test]
fn the_identity_survives_a_restart() {
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let store = Some(dir.path().join("dev-identity.pem"));
        let first = build_with_store(store.clone())
            .await
            .expect("first identity");
        let second = build_with_store(store).await.expect("second identity");
        assert_eq!(
            first.cert_hash_hex, second.cert_hash_hex,
            "a client's pinned link must survive a host restart"
        );
    });
}

#[test]
fn racing_cold_stores_never_leave_a_torn_pair() {
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("dev-identity.pem");
        let ready = std::sync::Arc::new(tokio::sync::Barrier::new(8));
        let racers: Vec<_> = (0..8)
            .map(|_| {
                let store = Some(store.clone());
                let ready = std::sync::Arc::clone(&ready);
                tokio::spawn(async move {
                    ready.wait().await;
                    let identity = build_with_store(store).await.unwrap();
                    drop(
                        wtransport::ServerConfig::builder()
                            .with_bind_address(([127, 0, 0, 1], 0).into())
                            .with_identity(identity.identity)
                            .build(),
                    );
                })
            })
            .collect();
        for racer in racers {
            racer.await.expect("racer task completes");
        }
        let settled = build_with_store(Some(store.clone()))
            .await
            .expect("the settled store loads");
        let reloaded = build_with_store(Some(store))
            .await
            .expect("the settled store loads again");
        assert_eq!(
            settled.cert_hash_hex, reloaded.cert_hash_hex,
            "the store must converge on one whole identity"
        );
        let server_config = wtransport::ServerConfig::builder()
            .with_bind_address(([127, 0, 0, 1], 0).into())
            .with_identity(settled.identity)
            .build();
        drop(server_config);
    });
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap()
}
