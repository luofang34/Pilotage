//! Reads the physical RadioMaster Pocket through `hidapi` (`--hid` path).
//!
//! This module owns I/O (blocking device reads), so it lives in this binary
//! rather than a `crates/` sans-IO core (ADR-0002).

use std::time::Instant;

use hidapi::{HidApi, HidDevice};
use pilotage_input::RawDeviceSample;
use pilotage_timing::MonoTimestamp;

use crate::error::ProbeError;
use crate::hid_decode::decode_report;

/// RadioMaster Pocket USB vendor ID (matches `tools/hid-probe`'s verified
/// value for the physical unit).
const TARGET_VENDOR_ID: u16 = 0x1209;
/// RadioMaster Pocket USB product ID.
const TARGET_PRODUCT_ID: u16 = 0x4F54;
/// Report buffer size: generously oversized for the 19-byte input report.
const REPORT_BUF_LEN: usize = 64;
/// Per-read timeout in milliseconds; short so a stale report never blocks a
/// send tick past its deadline.
const READ_TIMEOUT_MS: i32 = 5;
/// Axis count in a neutral (no-report-yet) sample: matches the RadioMaster
/// Pocket profile's 8 configured axes.
const NEUTRAL_AXIS_COUNT: usize = 8;

/// An open RadioMaster Pocket HID handle plus the clock it timestamps
/// samples from.
pub struct HidSource {
    device: HidDevice,
    start: Instant,
    /// Kept alive only because `HidDevice` borrows from it; dropping this
    /// would invalidate `device`.
    _api: HidApi,
}

impl HidSource {
    /// Opens the RadioMaster Pocket via `hidapi`, timing samples from
    /// `start`.
    ///
    /// # Errors
    ///
    /// Returns [`ProbeError::Hid`] if the backend fails to initialize or the
    /// device fails to open.
    pub fn open(start: Instant) -> Result<Self, ProbeError> {
        let api = HidApi::new()?;
        let device = api.open(TARGET_VENDOR_ID, TARGET_PRODUCT_ID)?;
        Ok(Self {
            device,
            start,
            _api: api,
        })
    }

    /// Reads one HID report (if available within the timeout) and decodes
    /// it; on timeout, returns a neutral all-zero sample stamped at the
    /// current elapsed time so the control loop's cadence is never gated on
    /// device liveness.
    ///
    /// # Errors
    ///
    /// Returns [`ProbeError::Hid`] if a HID read fails outright (not merely
    /// times out).
    pub fn sample(&self) -> Result<RawDeviceSample, ProbeError> {
        let mut buf = [0u8; REPORT_BUF_LEN];
        let sampled_at = elapsed_to_timestamp(self.start.elapsed());
        let read = self.device.read_timeout(&mut buf, READ_TIMEOUT_MS)?;
        if read == 0 {
            return Ok(RawDeviceSample::new(
                vec![0.0; NEUTRAL_AXIS_COUNT],
                0,
                sampled_at,
            ));
        }
        Ok(decode_report(&buf[..read], sampled_at))
    }
}

/// Converts an elapsed wall-clock duration into a [`MonoTimestamp`],
/// saturating rather than truncating on the (practically unreachable) case
/// of a run longer than `u64::MAX` nanoseconds.
#[allow(clippy::cast_possible_truncation)]
pub fn elapsed_to_timestamp(elapsed: std::time::Duration) -> MonoTimestamp {
    MonoTimestamp::from_nanos(elapsed.as_nanos().min(u128::from(u64::MAX)) as u64)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::elapsed_to_timestamp;
    use std::time::Duration;

    #[test]
    fn elapsed_to_timestamp_converts_nanos() {
        let ts = elapsed_to_timestamp(Duration::from_millis(5));
        assert_eq!(ts.as_nanos(), 5_000_000);
    }
}
