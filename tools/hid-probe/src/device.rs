//! Target device identity for the `read`/`capture` subcommands.

/// RadioMaster Pocket USB vendor ID.
pub const TARGET_VENDOR_ID: u16 = 0x1209;
/// RadioMaster Pocket USB product ID.
pub const TARGET_PRODUCT_ID: u16 = 0x4F54;
/// Number of 16-bit RadioMaster axis fields.
pub const AXIS_COUNT: usize = 8;
