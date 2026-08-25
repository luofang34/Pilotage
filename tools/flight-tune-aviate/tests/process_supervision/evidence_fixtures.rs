use std::io::Write as _;
use std::path::{Path, PathBuf};

use flight_tune_aviate::{ProcessStartIdentity, SupervisionAttestation};
use sha2::{Digest as _, Sha256};

pub(crate) fn digest_bytes(bytes: &[u8]) -> flight_tune::Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    flight_tune::Digest::from_bytes(hasher.finalize().into())
}

pub(crate) fn add_linked_temporary(destination: &Path, counter: u64) -> PathBuf {
    let path = destination
        .parent()
        .expect("document has a parent")
        .join(temporary_name(counter));
    std::fs::hard_link(destination, &path).expect("create linked publication state");
    path
}

pub(crate) fn add_unlinked_temporary(root: &Path, counter: u64) -> PathBuf {
    use std::os::unix::fs::OpenOptionsExt as _;

    let path = root.join(temporary_name(counter));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .expect("create unlinked temporary state");
    file.write_all(b"uncommitted recovery object")
        .expect("write unlinked temporary state");
    file.sync_all().expect("sync unlinked temporary state");
    path
}

pub(crate) fn add_conflicting_recovery_receipt(
    root: &Path,
    attestation: &SupervisionAttestation,
) -> PathBuf {
    use std::os::unix::fs::OpenOptionsExt as _;

    let prior_boot = boot_identity_json(&attestation.supervisor_identity.start);
    let bytes = format!(
        concat!(
            "{{\"schema_version\":1,",
            "\"run_intent_digest\":{},",
            "\"spawn_intent_digest\":{},",
            "\"process_identity_digest\":{},",
            "\"target_attestation_digest\":null,",
            "\"prior_boot_identity\":{},",
            "\"recovery_boot_identity\":{}}}"
        ),
        serde_json::to_string(&attestation.run_intent_digest).expect("encode run digest"),
        serde_json::to_string(&attestation.spawn_intent_digest).expect("encode spawn digest"),
        serde_json::to_string(&attestation.process_identity_digest).expect("encode process digest"),
        prior_boot,
        prior_boot,
    );
    let path = root.join("supervisor-recovery-receipt.json");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .expect("create conflicting recovery receipt");
    file.write_all(bytes.as_bytes())
        .expect("write conflicting recovery receipt");
    file.sync_all().expect("sync conflicting recovery receipt");
    path
}

pub(crate) fn add_unknown_storage_object(root: &Path) -> PathBuf {
    use std::os::unix::fs::OpenOptionsExt as _;

    let path = root.join("unknown-object.json");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .expect("create unknown storage object");
    file.write_all(b"{}").expect("write unknown storage object");
    file.sync_all().expect("sync unknown storage object");
    path
}

pub(crate) fn replace_file_bytes(path: &Path, bytes: &[u8]) {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .expect("open test document for replacement");
    file.write_all(bytes).expect("replace test document bytes");
    file.sync_all().expect("sync replaced test document");
}

fn boot_identity_json(start: &ProcessStartIdentity) -> String {
    match start {
        ProcessStartIdentity::Linux { boot_id, .. } => format!(
            "{{\"platform\":\"linux\",\"boot_id\":{}}}",
            serde_json::to_string(boot_id).expect("encode Linux boot identity")
        ),
        ProcessStartIdentity::MacOs {
            boot_session_uuid, ..
        } => format!(
            "{{\"platform\":\"mac_os\",\"boot_session_uuid\":{}}}",
            serde_json::to_string(boot_session_uuid).expect("encode Darwin boot identity")
        ),
    }
}

fn temporary_name(counter: u64) -> String {
    format!(".pilotage-tmp-{}-{counter:016x}", std::process::id())
}
