//! `list` subcommand: enumerate all connected HID devices.

use hidapi::HidApi;

use crate::error::ProbeError;
use crate::output::print_line;

/// Enumerates every HID device visible to the OS and prints vendor id,
/// product id, and product string, one per line.
///
/// # Errors
///
/// Returns [`ProbeError::Hid`] if the `hidapi` backend fails to initialize.
pub fn run() -> Result<(), ProbeError> {
    let api = HidApi::new()?;
    for device in api.device_list() {
        let product = device.product_string().unwrap_or("<unknown product>");
        print_line(&format!(
            "vendor={:#06x} product={:#06x} product_string=\"{product}\" path={}",
            device.vendor_id(),
            device.product_id(),
            device.path().to_string_lossy()
        ));
    }
    Ok(())
}
