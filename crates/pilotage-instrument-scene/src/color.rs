//! RGBA color carried by paint commands.

/// An 8-bit-per-channel RGBA color.
///
/// Encoded on the wire as a little-endian `u32` in `0xAABBGGRR` layout
/// (byte order r, g, b, a), so the byte stream reads in channel order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba8 {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel; 255 is opaque.
    pub a: u8,
}

impl Rgba8 {
    /// An opaque color from red, green, and blue channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// A color from all four channels.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Packs into the wire `u32` (byte order r, g, b, a when written
    /// little-endian).
    pub const fn to_u32(self) -> u32 {
        (self.r as u32) | ((self.g as u32) << 8) | ((self.b as u32) << 16) | ((self.a as u32) << 24)
    }

    /// Unpacks from the wire `u32`.
    pub const fn from_u32(v: u32) -> Self {
        Self {
            r: (v & 0xff) as u8,
            g: ((v >> 8) & 0xff) as u8,
            b: ((v >> 16) & 0xff) as u8,
            a: ((v >> 24) & 0xff) as u8,
        }
    }
}

#[cfg(test)]
mod tests;
