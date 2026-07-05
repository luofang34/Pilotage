//! Target device identity for the `read`/`capture` subcommands.

/// RadioMaster Pocket USB vendor ID (verified against the physical unit
/// connected during development; see task record for the enumeration
/// output).
pub const TARGET_VENDOR_ID: u16 = 0x1209;
/// RadioMaster Pocket USB product ID.
pub const TARGET_PRODUCT_ID: u16 = 0x4F54;
