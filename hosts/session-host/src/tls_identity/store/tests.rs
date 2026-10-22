use super::*;

#[test]
fn one_snapshot_keeps_its_key_when_the_store_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let pair = dir.path().join("identity.pem");
    let first = Identity::self_signed(["localhost"]).unwrap();
    let second = Identity::self_signed(["localhost"]).unwrap();
    persist_blocking(&first, &pair);
    let snapshot = std::fs::read(&pair).unwrap();
    persist_blocking(&second, &pair);
    let identity = decode(&snapshot).unwrap();
    assert_eq!(
        identity.certificate_chain().as_slice()[0].hash(),
        first.certificate_chain().as_slice()[0].hash()
    );
    drop(
        wtransport::ServerConfig::builder()
            .with_bind_default(0)
            .with_identity(identity)
            .build(),
    );
}

#[test]
fn a_mismatched_persisted_pair_is_rejected_before_server_configuration() {
    let first = Identity::self_signed(["localhost"]).unwrap();
    let second = Identity::self_signed(["localhost"]).unwrap();
    let mismatched = format!(
        "{}{}",
        first.private_key().to_secret_pem(),
        second.certificate_chain().as_slice()[0].to_pem()
    );
    assert!(decode(mismatched.as_bytes()).is_err());
}
