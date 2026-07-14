//! Decoders for the tile payload formats, the inverse of the encoders.
//!
//! An independent verifier decodes a produced package with these to re-derive
//! what the package actually contains, rather than trusting the pipeline's own
//! counters. Every decoder is bounds-checked and returns `None` on a malformed
//! or truncated payload, so a corrupt tile is a decode failure, never a panic.

/// A decoded terrain post: its grid indices and elevation.
pub(crate) struct DecodedPost {
    /// Global grid row index.
    pub i: u32,
    /// Global grid column index.
    pub j: u32,
    /// Elevation, target vertical datum, meters.
    pub elevation_m: f64,
}

/// A decoded obstacle position (the fields the verifier re-tiles against).
pub(crate) struct DecodedObstacle {
    /// Latitude, degrees.
    pub lat_deg: f64,
    /// Longitude, degrees.
    pub lon_deg: f64,
}

/// Reads a `u32` little-endian at `off`.
fn read_u32(bytes: &[u8], off: usize) -> Option<u32> {
    bytes
        .get(off..off + 4)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

/// Reads a `u64` little-endian at `off`.
fn read_u64(bytes: &[u8], off: usize) -> Option<u64> {
    bytes
        .get(off..off + 8)?
        .try_into()
        .ok()
        .map(u64::from_le_bytes)
}

/// Reads an `f64` (IEEE-754 bit pattern) little-endian at `off`.
fn read_f64(bytes: &[u8], off: usize) -> Option<f64> {
    read_u64(bytes, off).map(f64::from_bits)
}

/// The element count of a length-prefixed payload with `magic`, or `None` if the
/// magic or header is wrong.
fn header(bytes: &[u8], magic: &[u8; 8]) -> Option<u64> {
    if bytes.get(0..8)? != magic {
        return None;
    }
    read_u64(bytes, 8)
}

/// Decodes a terrain tile payload into its posts.
pub(crate) fn decode_terrain(bytes: &[u8]) -> Option<Vec<DecodedPost>> {
    let count = header(bytes, b"SVSBTERR")?;
    let mut out = Vec::new();
    let mut off = 16usize;
    for _ in 0..count {
        let i = read_u32(bytes, off)?;
        let j = read_u32(bytes, off + 4)?;
        let elevation_m = read_f64(bytes, off + 8)?;
        out.push(DecodedPost { i, j, elevation_m });
        off += 16;
    }
    Some(out)
}

/// Decodes an obstacle tile payload into its obstacles.
pub(crate) fn decode_obstacles(bytes: &[u8]) -> Option<Vec<DecodedObstacle>> {
    let count = header(bytes, b"SVSBOBST")?;
    let mut out = Vec::new();
    let mut off = 16usize;
    for _ in 0..count {
        let lat_deg = read_f64(bytes, off)?;
        let lon_deg = read_f64(bytes, off + 8)?;
        // Validate the height and kind bytes are present without storing them.
        read_f64(bytes, off + 16)?;
        bytes.get(off + 24)?;
        out.push(DecodedObstacle { lat_deg, lon_deg });
        off += 25;
    }
    Some(out)
}

/// The element count of an aerodrome tile payload, or `None` if malformed. The
/// records themselves are not needed by the verifier beyond their count.
pub(crate) fn decode_aerodrome_count(bytes: &[u8]) -> Option<u64> {
    let count = header(bytes, b"SVSBAERO")?;
    let end = 16usize.checked_add((count as usize).checked_mul(28)?)?;
    if bytes.len() < end {
        return None;
    }
    Some(count)
}

/// The element count of a runway tile payload, or `None` if malformed.
pub(crate) fn decode_runway_count(bytes: &[u8]) -> Option<u64> {
    let count = header(bytes, b"SVSBRUNW")?;
    let end = 16usize.checked_add((count as usize).checked_mul(34)?)?;
    if bytes.len() < end {
        return None;
    }
    Some(count)
}
