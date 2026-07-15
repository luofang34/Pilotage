//! Decoders for the tile payload formats, the inverse of the encoders.
//!
//! An independent verifier decodes a produced package with these to re-derive
//! what the package actually contains — its records and their identities —
//! rather than trusting the pipeline's own counters or lineage. Every decoder is
//! bounds-checked and returns `None` on a malformed or truncated payload, so a
//! corrupt tile is a decode failure, never a panic.

/// A decoded terrain post: its grid indices and elevation.
pub(crate) struct DecodedPost {
    /// Global grid row index.
    pub i: u32,
    /// Global grid column index.
    pub j: u32,
    /// Elevation, target vertical datum, meters.
    pub elevation_m: f64,
}

/// A decoded obstacle: position and kind (the identity fields).
pub(crate) struct DecodedObstacle {
    /// Latitude, degrees.
    pub lat_deg: f64,
    /// Longitude, degrees.
    pub lon_deg: f64,
    /// Kind wire code.
    pub kind: u8,
}

/// A decoded aerodrome: identifier and reference position.
pub(crate) struct DecodedAerodrome {
    /// The aerodrome identifier.
    pub ident: u32,
    /// Reference-point latitude, degrees.
    pub lat_deg: f64,
    /// Reference-point longitude, degrees.
    pub lon_deg: f64,
}

/// A decoded runway: designator and end-A position.
pub(crate) struct DecodedRunway {
    /// Runway designator.
    pub designator: u16,
    /// End-A latitude, degrees.
    pub end_a_lat_deg: f64,
    /// End-A longitude, degrees.
    pub end_a_lon_deg: f64,
}

/// Reads a `u16` little-endian at `off`.
fn read_u16(bytes: &[u8], off: usize) -> Option<u16> {
    bytes
        .get(off..off + 2)?
        .try_into()
        .ok()
        .map(u16::from_le_bytes)
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
        read_f64(bytes, off + 16)?; // height, present but not needed
        let kind = *bytes.get(off + 24)?;
        out.push(DecodedObstacle {
            lat_deg,
            lon_deg,
            kind,
        });
        off += 25;
    }
    Some(out)
}

/// Decodes an aerodrome tile payload into its reference points.
pub(crate) fn decode_aerodromes(bytes: &[u8]) -> Option<Vec<DecodedAerodrome>> {
    let count = header(bytes, b"SVSBAERO")?;
    let mut out = Vec::new();
    let mut off = 16usize;
    for _ in 0..count {
        let ident = read_u32(bytes, off)?;
        let lat_deg = read_f64(bytes, off + 4)?;
        let lon_deg = read_f64(bytes, off + 12)?;
        read_f64(bytes, off + 20)?; // elevation, present but not needed
        out.push(DecodedAerodrome {
            ident,
            lat_deg,
            lon_deg,
        });
        off += 28;
    }
    Some(out)
}

/// Decodes a runway tile payload into its runways.
pub(crate) fn decode_runways(bytes: &[u8]) -> Option<Vec<DecodedRunway>> {
    let count = header(bytes, b"SVSBRUNW")?;
    let mut out = Vec::new();
    let mut off = 16usize;
    for _ in 0..count {
        let designator = read_u16(bytes, off)?;
        let end_a_lat_deg = read_f64(bytes, off + 2)?;
        let end_a_lon_deg = read_f64(bytes, off + 10)?;
        read_f64(bytes, off + 18)?; // end-B latitude, present but not needed
        read_f64(bytes, off + 26)?; // end-B longitude, present but not needed
        out.push(DecodedRunway {
            designator,
            end_a_lat_deg,
            end_a_lon_deg,
        });
        off += 34;
    }
    Some(out)
}
