//! `read` subcommand: open the target device and print a live decode of
//! each input report for a fixed duration.
//!
//! This binary is a native diagnostic tool, not a sans-IO core crate
//! (ADR-0002 scopes that ban to `crates/`), so wall-clock reads via
//! `std::time::Instant` are in scope here.

use std::time::{Duration, Instant};

use hidapi::HidApi;

use crate::decode::{le_u16_words, to_hex};
use crate::device::{TARGET_PRODUCT_ID, TARGET_VENDOR_ID};
use crate::error::ProbeError;
use crate::output::print_line;

/// Report buffer size: generously oversized for a joystick HID input
/// report; `read_timeout` returns the actual byte count read.
pub(crate) const REPORT_BUF_LEN: usize = 64;
/// Per-read timeout in milliseconds; short enough that the overall
/// `seconds` budget is respected without a long final blocking read.
const READ_TIMEOUT_MS: i32 = 200;

/// Opens the RadioMaster Pocket and prints a timestamped hex + decoded
/// little-endian `u16` view of every input report received in `seconds`
/// wall-clock seconds.
///
/// # Errors
///
/// Returns [`ProbeError::Hid`] if the backend fails to initialize or the
/// device fails to open or read.
pub fn run(seconds: u64) -> Result<(), ProbeError> {
    let api = HidApi::new()?;
    let device = api.open(TARGET_VENDOR_ID, TARGET_PRODUCT_ID)?;
    let start = Instant::now();
    let budget = Duration::from_secs(seconds);
    let mut buf = [0u8; REPORT_BUF_LEN];
    while start.elapsed() < budget {
        let read = device.read_timeout(&mut buf, READ_TIMEOUT_MS)?;
        if read == 0 {
            continue;
        }
        let bytes = &buf[..read];
        print_line(&format!(
            "t={:>6}ms hex=[{}] words={:?}",
            start.elapsed().as_millis(),
            to_hex(bytes),
            le_u16_words(bytes)
        ));
    }
    Ok(())
}
