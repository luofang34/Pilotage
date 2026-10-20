use std::io::{self, Write};
use std::time::Duration;

/// The timestamps describe host receipt and write acceptance, not device time.
pub(super) fn record_blocking(
    destination: &mut dyn Write,
    elapsed: Duration,
    direction: &str,
    bytes: &[u8],
) -> io::Result<()> {
    let mut record = format!(
        "{{\"elapsed_ns\":{},\"direction\":\"{direction}\",\"hex\":\"",
        elapsed.as_nanos()
    );
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        record.push(char::from(HEX[usize::from(byte >> 4)]));
        record.push(char::from(HEX[usize::from(byte & 15)]));
    }
    record.push_str("\"}\n");
    destination.write_all(record.as_bytes())?;
    destination.flush()
}
