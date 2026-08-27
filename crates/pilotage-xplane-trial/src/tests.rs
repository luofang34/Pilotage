#![allow(clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::*;
use crate::identity::file_digest_blocking;

struct Fixture {
    root: PathBuf,
    aircraft: PathBuf,
    trial: PathBuf,
    bridge: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pilotage-xplane-trial-{}-{nonce}",
            std::process::id()
        ));
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

    fn expected(&self) -> ExpectedXPlaneIdentity {
        ExpectedXPlaneIdentity {
            aircraft: expected("aircraft", &self.aircraft),
            trial_plugin: expected("trial", &self.trial),
            bridge_plugin: expected("bridge", &self.bridge),
            bridge_config: expected("config", &self.config),
            trial_source_build_id: "source-build".to_owned(),
            simulator_model_digest: Digest::from_bytes([9; 32]),
        }
    }

    fn hello(&self) -> String {
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

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

#[test]
fn a_verified_session_binds_identity_and_checks_every_sample() {
    let fixture = Fixture::new();
    let expected = fixture.expected();
    let listener = XPlaneTrialListener::bind_blocking("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let hello = fixture.hello();
    let peer = thread::spawn(move || fake_complete_trial(address, &hello));

    let mut session = listener
        .accept_verified_blocking(&expected, Duration::from_secs(2))
        .expect("verified session");
    assert_eq!(
        session.identity().simulator_model_digest,
        expected.simulator_model_digest
    );
    assert!(!session.identity().binding_digest.is_zero());
    let scenario = Digest::from_bytes([1; 32]);
    let condition = Digest::from_bytes([2; 32]);
    assert!(matches!(
        session
            .configure_blocking(2, scenario, condition)
            .expect("configure"),
        SessionReceipt::Configured { generation: 2 }
    ));
    assert!(matches!(
        session
            .set_wind_blocking(
                1,
                pilotage_trial::AppliedWind {
                    speed_mps: 4.0,
                    direction_deg: 90.0,
                    north_mps: 0.0,
                    east_mps: -4.0,
                    turbulence_mps: 0.0,
                },
            )
            .expect("wind"),
        SessionReceipt::WindApplied {
            generation: 2,
            condition_generation: 1,
            actual_speed_mps: 4.1,
            actual_direction_deg: 91.0,
            ..
        }
    ));
    session.start_blocking().expect("start");
    assert_active_wind_buffers_an_earlier_sample(&mut session);
    assert_eq!(
        session
            .next_sample_blocking()
            .expect("sample 0")
            .position_ned_m(),
        [0.0, 0.0, 0.0]
    );
    assert_eq!(
        session.next_sample_blocking().expect("sample 1").sequence,
        1
    );
    assert!(matches!(
        session.stop_blocking().expect("stop"),
        SessionReceipt::Stopped {
            generation: 2,
            sample_count: 2,
            ..
        }
    ));
    assert!(matches!(
        session.reset_blocking(3).expect("reset"),
        SessionReceipt::ResetComplete {
            generation: 3,
            reset_generation: 1,
            sim_time_s: 0.0,
        }
    ));
    peer.join().expect("peer");
}

fn assert_active_wind_buffers_an_earlier_sample(session: &mut XPlaneTrialSession) {
    let receipt = session
        .set_wind_blocking(
            2,
            pilotage_trial::AppliedWind {
                speed_mps: 5.0,
                direction_deg: 180.0,
                north_mps: 5.0,
                east_mps: 0.0,
                turbulence_mps: 0.0,
            },
        )
        .expect("active wind");
    assert!(matches!(
        receipt,
        SessionReceipt::WindApplied {
            generation: 2,
            condition_generation: 2,
            actual_speed_mps: 5.1,
            actual_direction_deg: 181.0,
            ..
        }
    ));
}

#[test]
fn a_changed_aircraft_file_is_rejected_before_configuration() {
    let fixture = Fixture::new();
    let mut expected = fixture.expected();
    expected.aircraft = ExpectedArtifact::new(&fixture.aircraft, Digest::from_bytes([7; 32]));
    let listener = XPlaneTrialListener::bind_blocking("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let hello = fixture.hello();
    let peer = thread::spawn(move || {
        let mut stream = TcpStream::connect(address).expect("connect");
        stream.write_all(hello.as_bytes()).expect("hello");
    });

    let result = listener.accept_verified_blocking(&expected, Duration::from_secs(2));

    assert!(matches!(
        result,
        Err(XPlaneTrialError::ArtifactDigest { .. })
    ));
    peer.join().expect("peer");
}

#[test]
fn a_loaded_trial_source_mismatch_is_rejected() {
    let fixture = Fixture::new();
    let expected = fixture.expected();
    let hello = fixture
        .hello()
        .replace(&hex("source-build"), &hex("other-build"));

    let result = verify_hello(&expected, hello);

    assert!(matches!(
        result,
        Err(XPlaneTrialError::TrialSourceBuild { .. })
    ));
}

#[test]
fn a_loaded_bridge_bundle_mismatch_is_rejected() {
    let fixture = Fixture::new();
    let expected = fixture.expected();
    let digest = file_digest_blocking("bridge", &fixture.bridge)
        .expect("bridge digest")
        .to_string();
    let hello = fixture.hello().replacen(&digest, &"7".repeat(64), 1);

    let result = verify_hello(&expected, hello);

    assert!(matches!(
        result,
        Err(XPlaneTrialError::LoadedBridgeDigest { .. })
    ));
}

#[test]
fn legacy_hfs_paths_must_match_the_expected_posix_components() {
    let fixture = Fixture::new();
    let expected = fixture.expected();
    let hello = format!(
        "HELLO 2 120400 400 1 {} {} {} {} {}\n",
        hex("source-build"),
        file_digest_blocking("bridge", &fixture.bridge).expect("bridge digest"),
        hex(&legacy_hfs_path(&fixture.aircraft)),
        hex(&legacy_hfs_path(&fixture.trial)),
        hex(&legacy_hfs_path(&fixture.bridge)),
    );

    let session = verify_hello(&expected, hello).expect("verified HFS paths");

    assert_eq!(
        session.identity().aircraft_digest,
        expected.aircraft.digest()
    );
}

#[test]
fn a_sequence_gap_fails_the_stream() {
    let fixture = Fixture::new();
    let expected = fixture.expected();
    let listener = XPlaneTrialListener::bind_blocking("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let hello = fixture.hello();
    let peer = thread::spawn(move || fake_gap_trial(address, &hello));
    let mut session = listener
        .accept_verified_blocking(&expected, Duration::from_secs(2))
        .expect("verified session");
    session
        .configure_blocking(3, Digest::from_bytes([3; 32]), Digest::from_bytes([4; 32]))
        .expect("configure");
    session.start_blocking().expect("start");

    let result = session.next_sample_blocking();

    assert!(matches!(
        result,
        Err(XPlaneTrialError::ReceiptMismatch { .. })
    ));
    peer.join().expect("peer");
}

#[test]
fn each_sample_read_enforces_its_own_wall_timeout() {
    let fixture = Fixture::new();
    let expected = fixture.expected();
    let listener = XPlaneTrialListener::bind_blocking("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let hello = fixture.hello();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let peer = thread::spawn(move || {
        fake_silent_trial(address, &hello, ready_tx, release_rx);
    });
    let mut session = listener
        .accept_verified_blocking(&expected, Duration::from_secs(2))
        .expect("verified session");
    session
        .configure_blocking(6, Digest::from_bytes([6; 32]), Digest::from_bytes([7; 32]))
        .expect("configure");
    session.start_blocking().expect("start");
    ready_rx.recv().expect("peer ready");

    let started = Instant::now();
    let result = session.next_sample_with_timeout_blocking(Duration::from_millis(30));
    let elapsed = started.elapsed();

    assert!(matches!(
        result,
        Err(XPlaneTrialError::SessionIo { ref source, .. })
            if matches!(source.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)
    ));
    assert!(elapsed < Duration::from_millis(250));
    release_tx.send(()).expect("release peer");
    peer.join().expect("peer");
}

#[test]
fn a_non_local_listener_is_rejected() {
    let result = XPlaneTrialListener::bind_blocking("0.0.0.0:0");

    assert!(matches!(
        result,
        Err(XPlaneTrialError::NonLocalAddress { .. })
    ));
}

#[test]
fn an_installed_build_manifest_is_strict_and_validated() {
    let fixture = Fixture::new();
    let path = fixture.root.join("build-manifest.json");
    let digest = Digest::from_bytes([5; 32]);
    std::fs::write(
        &path,
        format!(
            "{{\"schema_version\":1,\"trial_source_build_id\":\"source\",\
             \"bridge_plugin_digest\":\"{digest}\"}}"
        ),
    )
    .expect("manifest");
    let manifest = TrialPluginBuildManifest::from_json_file_blocking(&path).expect("valid");
    assert_eq!(manifest.bridge_plugin_digest, digest);

    std::fs::write(
        &path,
        format!(
            "{{\"schema_version\":1,\"trial_source_build_id\":\"source\",\
             \"bridge_plugin_digest\":\"{digest}\",\"unknown\":true}}"
        ),
    )
    .expect("unknown manifest");
    assert!(matches!(
        TrialPluginBuildManifest::from_json_file_blocking(&path),
        Err(XPlaneTrialError::BuildManifestDecode { .. })
    ));
}

#[test]
fn a_binding_rejects_a_changed_aircraft_identity() {
    let fixture = Fixture::new();
    let expected = fixture.expected();
    let session = verify_hello(&expected, fixture.hello()).expect("verified identity");
    assert!(session.identity().binding_is_valid());

    let mut changed = session.identity().clone();
    changed.aircraft_digest = Digest::from_bytes([99; 32]);

    assert!(!changed.binding_is_valid());
}

fn verify_hello(
    expected: &ExpectedXPlaneIdentity,
    hello: String,
) -> Result<XPlaneTrialSession, XPlaneTrialError> {
    let listener = XPlaneTrialListener::bind_blocking("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let peer = thread::spawn(move || {
        let mut stream = TcpStream::connect(address).expect("connect");
        stream.write_all(hello.as_bytes()).expect("hello");
    });
    let result = listener.accept_verified_blocking(expected, Duration::from_secs(2));
    peer.join().expect("peer");
    result
}

fn fake_complete_trial(address: std::net::SocketAddr, hello: &str) {
    let mut stream = TcpStream::connect(address).expect("connect");
    stream.write_all(hello.as_bytes()).expect("hello");
    let reader_stream = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(reader_stream);
    let config = read_command(&mut reader);
    let fields = config.split_ascii_whitespace().collect::<Vec<_>>();
    writeln!(
        stream,
        "CONFIGURED {} {} {}",
        fields[1], fields[2], fields[3]
    )
    .expect("configured");
    assert_eq!(read_command(&mut reader), "WIND 2 1 4 90");
    writeln!(stream, "WIND_APPLIED 2 1 4.1 91").expect("wind applied");
    assert_eq!(read_command(&mut reader), "START 2");
    writeln!(stream, "STARTED 2 1.0 0").expect("started");
    writeln!(stream, "{}", sample(2, 0, 1.00)).expect("sample 0");
    assert_eq!(read_command(&mut reader), "WIND 2 2 5 180");
    writeln!(stream, "WIND_APPLIED 2 2 5.1 181").expect("active wind applied");
    writeln!(stream, "{}", sample(2, 1, 1.01)).expect("sample 1");
    assert_eq!(read_command(&mut reader), "STOP 2");
    writeln!(stream, "STOPPED 2 2 1.02").expect("stopped");
    assert_eq!(read_command(&mut reader), "RESET 3");
    writeln!(stream, "RESETTING 3").expect("resetting");
    writeln!(stream, "RESET_COMPLETE 3 1 0").expect("reset complete");
}

fn fake_gap_trial(address: std::net::SocketAddr, hello: &str) {
    let mut stream = TcpStream::connect(address).expect("connect");
    stream.write_all(hello.as_bytes()).expect("hello");
    let reader_stream = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(reader_stream);
    let config = read_command(&mut reader);
    let fields = config.split_ascii_whitespace().collect::<Vec<_>>();
    writeln!(
        stream,
        "CONFIGURED {} {} {}",
        fields[1], fields[2], fields[3]
    )
    .expect("configured");
    assert_eq!(read_command(&mut reader), "START 3");
    writeln!(stream, "STARTED 3 1.0 0").expect("started");
    writeln!(stream, "{}", sample(3, 1, 1.01)).expect("sample gap");
}

fn fake_silent_trial(
    address: std::net::SocketAddr,
    hello: &str,
    ready: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
) {
    let mut stream = TcpStream::connect(address).expect("connect");
    stream.write_all(hello.as_bytes()).expect("hello");
    let reader_stream = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(reader_stream);
    let config = read_command(&mut reader);
    let fields = config.split_ascii_whitespace().collect::<Vec<_>>();
    writeln!(
        stream,
        "CONFIGURED {} {} {}",
        fields[1], fields[2], fields[3]
    )
    .expect("configured");
    assert_eq!(read_command(&mut reader), "START 6");
    writeln!(stream, "STARTED 6 1.0 0").expect("started");
    ready.send(()).expect("ready");
    release.recv().expect("release");
}

fn sample(generation: u64, sequence: u64, sim_time: f64) -> String {
    format!(
        "SAMPLE {generation} {sequence} {sim_time} 0.01 0 0 0 0 0 0 0 0 0 0 0 0 1 1 0 0 0 0 0 0 1 0 0 0"
    )
}

fn read_command(reader: &mut BufReader<TcpStream>) -> String {
    let mut line = String::new();
    reader.read_line(&mut line).expect("read command");
    line.trim().to_owned()
}

fn expected(name: &'static str, path: &Path) -> ExpectedArtifact {
    ExpectedArtifact::new(path, file_digest_blocking(name, path).expect("file digest"))
}

fn hex_path(path: &Path) -> String {
    hex(&path.to_string_lossy())
}

fn legacy_hfs_path(path: &Path) -> String {
    let canonical = std::fs::canonicalize(path).expect("canonical path");
    let components = canonical
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>();
    format!("Startup:{}", components.join(":"))
}

fn hex(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
