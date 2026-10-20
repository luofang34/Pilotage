//! Bounded RGB frames from the decoder's binary PPM output.

const MAX_HEADER: usize = 256;
const MAX_WIDTH: u32 = 4096;
const MAX_HEIGHT: u32 = 2160;

#[derive(Debug, thiserror::Error)]
pub enum PpmError {
    /// The decoder output was not an eight-bit RGB PPM frame.
    #[error("invalid RGB PPM header")]
    Header,
    /// The PPM header exceeded its byte limit.
    #[error("PPM header exceeds {MAX_HEADER} bytes")]
    HeaderLimit,
    /// The decoded dimensions exceeded the host's image limit.
    #[error("PPM dimensions {width}x{height} exceed the video limit")]
    Dimensions {
        /// The decoded width in pixels.
        width: u32,
        /// The decoded height in pixels.
        height: u32,
    },
    /// The stream ended inside a header or pixel buffer.
    #[error("decoder ended inside a PPM frame")]
    Truncated,
}

#[derive(Debug)]
pub(super) struct PpmFrame {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) rgb: Vec<u8>,
}

#[derive(Default)]
pub(super) struct PpmDecoder {
    header: Vec<u8>,
    frame: Option<PpmFrame>,
    remaining: usize,
}

impl PpmDecoder {
    pub(super) fn push(&mut self, mut bytes: &[u8]) -> Result<Vec<PpmFrame>, PpmError> {
        let mut frames = Vec::new();
        while !bytes.is_empty() {
            if let Some(frame) = &mut self.frame {
                let count = bytes.len().min(self.remaining);
                frame.rgb.extend_from_slice(&bytes[..count]);
                bytes = &bytes[count..];
                self.remaining -= count;
                if self.remaining == 0
                    && let Some(frame) = self.frame.take()
                {
                    frames.push(frame);
                }
            } else {
                let Some((&byte, rest)) = bytes.split_first() else {
                    break;
                };
                bytes = rest;
                if self.header.len() == MAX_HEADER {
                    return Err(PpmError::HeaderLimit);
                }
                self.header.push(byte);
                if byte.is_ascii_whitespace()
                    && let Some((width, height, size)) = parse_header(&self.header)?
                {
                    self.header.clear();
                    self.frame = Some(PpmFrame {
                        width,
                        height,
                        rgb: Vec::with_capacity(size),
                    });
                    self.remaining = size;
                }
            }
        }
        Ok(frames)
    }

    pub(super) fn finish(&self) -> Result<(), PpmError> {
        if self.header.is_empty() && self.frame.is_none() {
            Ok(())
        } else {
            Err(PpmError::Truncated)
        }
    }
}

fn parse_header(bytes: &[u8]) -> Result<Option<(u32, u32, usize)>, PpmError> {
    let text = std::str::from_utf8(bytes).map_err(|_| PpmError::Header)?;
    let mut fields = text.split_ascii_whitespace();
    if fields.next() != Some("P6") {
        return Err(PpmError::Header);
    }
    let (Some(width), Some(height), Some(depth)) = (fields.next(), fields.next(), fields.next())
    else {
        return Ok(None);
    };
    if depth != "255" || fields.next().is_some() {
        return Err(PpmError::Header);
    }
    let width = width.parse::<u32>().map_err(|_| PpmError::Header)?;
    let height = height.parse::<u32>().map_err(|_| PpmError::Header)?;
    if width == 0 || height == 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
        return Err(PpmError::Dimensions { width, height });
    }
    Ok(Some((width, height, width as usize * height as usize * 3)))
}

#[cfg(test)]
mod tests;
