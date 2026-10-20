/// One step of the MAVLink X.25 CRC.
fn crc_accumulate(byte: u8, crc: u16) -> u16 {
    let tmp = byte ^ (crc as u8);
    let tmp = tmp ^ (tmp << 4);
    (crc >> 8) ^ (u16::from(tmp) << 8) ^ (u16::from(tmp) << 3) ^ (u16::from(tmp) >> 4)
}

/// MAVLink CRC over `data`, finished with the message's CRC_EXTRA.
pub fn compute_crc(data: &[u8], extra: u8) -> u16 {
    let mut crc = 0xFFFFu16;
    for &b in data {
        crc = crc_accumulate(b, crc);
    }
    crc_accumulate(extra, crc)
}
