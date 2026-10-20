use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{Digest, ExpectedXPlaneIdentity, expected, file_digest_blocking, hex, hex_path};

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) aircraft: PathBuf,
    pub(super) trial: PathBuf,
    pub(super) bridge: PathBuf,
    config: PathBuf,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        Self::with_nonce_blocking(nonce)
    }

    fn with_nonce_blocking(nonce: u128) -> Self {
        let root = reserve_root_blocking(nonce).expect("exclusive fixture directory");
        let aircraft = root.join("Aircraft/Test Aircraft.acf");
        let trial = root.join("Resources/plugins/PilotageTrial/64/mac.xpl");
        let bridge = root.join("Resources/plugins/px4xplane/64/mac.xpl");
        let config = root.join("Resources/plugins/px4xplane/64/config.ini");
        for path in [&aircraft, &trial, &bridge, &config] {
            std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
            std::fs::write(path, path.to_string_lossy().as_bytes()).expect("file");
        }
        Self {
            root,
            aircraft,
            trial,
            bridge,
            config,
        }
    }

    pub(super) fn expected(&self) -> ExpectedXPlaneIdentity {
        ExpectedXPlaneIdentity {
            aircraft: expected("aircraft", &self.aircraft),
            trial_plugin: expected("trial", &self.trial),
            bridge_plugin: expected("bridge", &self.bridge),
            bridge_config: expected("config", &self.config),
            trial_source_build_id: "source-build".to_owned(),
            simulator_model_digest: Digest::from_bytes([9; 32]),
        }
    }

    pub(super) fn hello(&self) -> String {
        format!(
            "HELLO 2 120400 400 1 {} {} {} {} {}\n",
            hex("source-build"),
            file_digest_blocking("bridge", &self.bridge).expect("bridge digest"),
            hex_path(&self.aircraft),
            hex_path(&self.trial),
            hex_path(&self.bridge),
        )
    }
}

fn reserve_root_blocking(nonce: u128) -> std::io::Result<PathBuf> {
    let prefix = std::env::temp_dir().join(format!(
        "pilotage-xplane-trial-{}-{nonce}",
        std::process::id()
    ));
    for attempt in 0..1024 {
        let root = prefix.with_extension(attempt.to_string());
        // Filesystem ownership, not clock resolution, permits this fixture's Drop to remove the root.
        match std::fs::create_dir(&root) {
            Ok(()) => return Ok(root),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!("no unused fixture directory for {}", prefix.display()),
    ))
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

#[test]
fn concurrent_fixtures_with_the_same_nonce_have_independent_cleanup() {
    let barrier = std::sync::Barrier::new(2);
    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            Fixture::with_nonce_blocking(0)
        });
        let second = scope.spawn(|| {
            barrier.wait();
            Fixture::with_nonce_blocking(0)
        });
        (first.join().expect("first"), second.join().expect("second"))
    });
    assert_ne!(first.root, second.root);
    let expected = second.expected();
    drop(first);
    let verified = super::verify_hello(&expected, second.hello()).expect("surviving fixture");
    assert_eq!(
        verified.identity().aircraft_digest,
        expected.aircraft.digest()
    );
}
