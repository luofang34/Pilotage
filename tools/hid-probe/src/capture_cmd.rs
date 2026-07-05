//! `capture` subcommand: record raw input reports from the target device to
//! a JSON fixture file for later use as a `pilotage-input` test fixture.

use std::path::Path;
use std::time::{Duration, Instant};

use hidapi::HidApi;
use serde::Serialize;

use crate::decode::to_hex;
use crate::device::{TARGET_PRODUCT_ID, TARGET_VENDOR_ID};
use crate::error::ProbeError;
use crate::read_cmd::REPORT_BUF_LEN;

/// Per-read timeout in milliseconds, matching `read_cmd`.
const READ_TIMEOUT_MS: i32 = 200;

/// One captured HID input report.
#[derive(Debug, Serialize)]
struct CapturedReport {
    /// Milliseconds since capture start when this report was read.
    t_ms: u128,
    /// Raw report bytes as lowercase hex pairs, e.g. `"01 ff 00"`.
    bytes_hex: String,
}

/// Device identity recorded alongside the captured reports.
#[derive(Debug, Serialize)]
struct CapturedDevice {
    /// USB vendor ID of the captured device.
    vendor_id: u16,
    /// USB product ID of the captured device.
    product_id: u16,
    /// Product string reported by the device, if any.
    product: Option<String>,
}

/// Top-level JSON shape written to the `--out` path.
#[derive(Debug, Serialize)]
struct Capture {
    /// Identity of the device the reports were captured from.
    device: CapturedDevice,
    /// Reports in capture order.
    reports: Vec<CapturedReport>,
}

/// Opens the RadioMaster Pocket, records input reports for `seconds`
/// wall-clock seconds, and writes them as JSON to `out`.
///
/// # Errors
///
/// Returns [`ProbeError::Hid`] if the backend fails to initialize, open, or
/// read the device; [`ProbeError::CaptureSerialize`] if the recorded
/// reports fail to serialize; [`ProbeError::CaptureWrite`] if writing `out`
/// fails.
pub fn run(seconds: u64, out: &Path) -> Result<(), ProbeError> {
    let api = HidApi::new()?;
    let device = api.open(TARGET_VENDOR_ID, TARGET_PRODUCT_ID)?;
    let product = device.get_product_string()?;
    let start = Instant::now();
    let budget = Duration::from_secs(seconds);
    let mut buf = [0u8; REPORT_BUF_LEN];
    let mut reports = Vec::new();
    while start.elapsed() < budget {
        let read = device.read_timeout(&mut buf, READ_TIMEOUT_MS)?;
        if read == 0 {
            continue;
        }
        reports.push(CapturedReport {
            t_ms: start.elapsed().as_millis(),
            bytes_hex: to_hex(&buf[..read]),
        });
    }
    let capture = Capture {
        device: CapturedDevice {
            vendor_id: TARGET_VENDOR_ID,
            product_id: TARGET_PRODUCT_ID,
            product,
        },
        reports,
    };
    write_capture(&capture, out)
}

/// Serializes `capture` as pretty JSON and writes it to `out`.
fn write_capture(capture: &Capture, out: &Path) -> Result<(), ProbeError> {
    let json = serde_json::to_string_pretty(capture)
        .map_err(|source| ProbeError::CaptureSerialize { source })?;
    std::fs::write(out, json).map_err(|source| ProbeError::CaptureWrite {
        path: out.to_path_buf(),
        source,
    })
}
