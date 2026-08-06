//! Golden-frame generation for the state ABI (`gen-state-fixture`).
//!
//! Encodes the shared posture fixtures with the same Rust codec the
//! runtime uses and writes one lowercase-hex line per fixture into
//! `crates/pilotage-instrument-state/fixtures/` — inside the crate that
//! owns the codec, so the fixtures travel with it. The Rust golden test
//! and the JS state-writer
//! test both pin against these committed files, so the two sides of the
//! wasm boundary can only drift by turning CI red.

use std::path::{Path, PathBuf};

use pilotage_instrument_state::AircraftState;
use pilotage_instrument_state::abi::v6::{CAPACITY, encode_state, fixtures};

use crate::error::XtaskError;
use crate::output::print_line;

/// Builds one posture fixture.
type FixtureBuilder = fn() -> AircraftState;

/// The committed fixtures: stable file stem and the state behind it.
const FIXTURES: [(&str, FixtureBuilder); 3] = [
    ("state-abi-v6.full", fixtures::full),
    ("state-abi-v6.data-gateway", fixtures::data_gateway),
    (
        "state-abi-v6.flight-controller",
        fixtures::flight_controller,
    ),
];

fn hex_of(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for byte in bytes {
        // Writing to a String cannot fail; ignore the fmt plumbing.
        write!(out, "{byte:02x}").ok();
    }
    out
}

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <repo>/tools/xtask.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Writes every golden frame, printing each path and byte count.
pub fn run() -> Result<(), XtaskError> {
    let dir = repo_root()
        .join("crates")
        .join("pilotage-instrument-state")
        .join("fixtures");
    std::fs::create_dir_all(&dir).map_err(|source| XtaskError::Io {
        context: "creating crates/pilotage-instrument-state/fixtures",
        source,
    })?;
    for (stem, build) in FIXTURES {
        let state = build();
        let mut buf = [0u8; CAPACITY];
        let len = encode_state(&state, &mut buf).map_err(|error| XtaskError::Usage {
            message: format!("encoding fixture {stem}: {error}"),
        })?;
        let path = dir.join(format!("{stem}.hex"));
        let mut content = hex_of(&buf[..len]);
        content.push('\n');
        std::fs::write(&path, content).map_err(|source| XtaskError::Io {
            context: "writing a state-ABI golden frame",
            source,
        })?;
        print_line(&format!("wrote {} ({len} bytes)", path.display()));
    }
    Ok(())
}
