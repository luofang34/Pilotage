//! H.264 Annex-B access-unit classification: the byte-level half of the
//! `H264` FourCC video path (ADR-0016 codec dispatch), shared by every
//! consumer that must decide what a chunk MEANS before handing it to a
//! platform decoder (WebCodecs in the browser, or any native decoder built
//! on the same classification). Decoding is never done here;
//! this module only reads NAL structure: keyframe determination, in-band
//! parameter-set presence, and the `avc1.PPCCLL` codec string derived from
//! the SPS.
//!
//! Fail closed: a keyframe that cannot configure a decoder (no usable SPS or
//! PPS preceding the IDR, or an SPS too short to name its profile)
//! classifies as undecodable with a typed reason, and input with no
//! recognizable NAL units at all classifies as invalid — the session layer
//! fails on it rather than feeding bytes no decoder can interpret.

/// FourCC the wire tags an H.264 Annex-B video body with (ADR-0016).
pub const FOURCC: [u8; 4] = *b"H264";

// H.264 `nal_unit_type` values (the header byte after each start code,
// masked with 0x1F): IDR slice = keyframe, and the two parameter sets a
// decoder needs before it can be configured.
const NAL_IDR: u8 = 5;
const NAL_SPS: u8 = 7;
const NAL_PPS: u8 = 8;

/// One NAL unit located in an Annex-B buffer: its `nal_unit_type`, the
/// offset of its start code, and the offset of its header byte (the byte
/// after the start code). The unit's bytes end where the next unit's start
/// code begins (or at the end of the buffer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NalUnit {
    /// The unit's `nal_unit_type` (header byte & 0x1F).
    pub nal_type: u8,
    /// Offset of the start code's first byte within the buffer.
    pub start_offset: usize,
    /// Offset of the NAL header byte within the buffer.
    pub header_offset: usize,
}

/// Iterates the NAL units of an Annex-B buffer, recognizing both the 3-byte
/// (`00 00 01`) and 4-byte (`00 00 00 01`) start codes. A start code at the
/// very end of the buffer carries no header byte and yields nothing.
#[derive(Debug, Clone)]
pub struct NalUnits<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Iterator for NalUnits<'_> {
    type Item = NalUnit;

    fn next(&mut self) -> Option<NalUnit> {
        let b = self.bytes;
        let n = b.len();
        while self.at + 3 <= n {
            let i = self.at;
            let start4 = i + 4 <= n && b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 0 && b[i + 3] == 1;
            let start3 = b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 1;
            if !(start4 || start3) {
                self.at += 1;
                continue;
            }
            let header = i + if start4 { 4 } else { 3 };
            self.at = header + 1;
            if let Some(byte) = b.get(header) {
                return Some(NalUnit {
                    nal_type: byte & 0x1f,
                    start_offset: i,
                    header_offset: header,
                });
            }
        }
        None
    }
}

/// The NAL units of `bytes`, in stream order.
#[must_use]
pub fn nal_units(bytes: &[u8]) -> NalUnits<'_> {
    NalUnits { bytes, at: 0 }
}

/// Why a keyframe cannot configure a decoder. A parameter set is usable
/// only when it precedes the IDR in stream order — a decoder consumes the
/// stream sequentially, so `IDR → SPS → PPS` configures nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyframeFault {
    /// No SPS precedes the IDR, so no profile/level is named for it.
    MissingSps,
    /// No PPS precedes the IDR, so its slice parameters are absent.
    MissingPps,
    /// The SPS ends before the three profile/constraint/level bytes.
    SpsTooShort,
}

impl KeyframeFault {
    /// A stable, human-readable reason for logs and typed failure surfaces.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::MissingSps => "no in-band SPS precedes the IDR",
            Self::MissingPps => "no in-band PPS precedes the IDR",
            Self::SpsTooShort => "SPS too short to name a profile",
        }
    }
}

/// What one access unit means to a decode session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkClass {
    /// Not an Annex-B access unit at all: no NAL units were found. Feeding
    /// such bytes to a decoder is never meaningful; the session fails on it.
    Invalid,
    /// No IDR slice: decodable only by an already-configured decoder.
    Delta,
    /// An IDR keyframe carrying both parameter sets in-band, with the
    /// `avc1.PPCCLL` codec string derived from its SPS — everything a
    /// decoder needs to (re)configure and decode from this chunk.
    Keyframe {
        /// The WebCodecs-style codec string (`avc1.` + the SPS
        /// profile_idc, constraint flags, and level_idc bytes in hex).
        codec: String,
    },
    /// An IDR keyframe that cannot configure a decoder; the session layer
    /// must fail visibly rather than feed it.
    UndecodableKeyframe {
        /// The typed reason configuration is impossible.
        fault: KeyframeFault,
    },
}

/// Classifies one access unit. Only parameter sets that PRECEDE the first
/// IDR count as usable — a decoder consumes the stream sequentially — and
/// the first such SPS names the codec, matching the parameter set a decoder
/// would adopt.
#[must_use]
pub fn classify_chunk(bytes: &[u8]) -> ChunkClass {
    let mut idr = false;
    let mut sps: Option<(usize, usize)> = None; // (header offset, unit end)
    let mut pps_before_idr = false;
    let mut units = nal_units(bytes).peekable();
    if units.peek().is_none() {
        return ChunkClass::Invalid;
    }
    while let Some(nal) = units.next() {
        let unit_end = units.peek().map_or(bytes.len(), |next| next.start_offset);
        match nal.nal_type {
            NAL_IDR => idr = true,
            NAL_SPS if !idr && sps.is_none() => sps = Some((nal.header_offset, unit_end)),
            NAL_PPS if !idr => pps_before_idr = true,
            _ => {}
        }
    }
    if !idr {
        return ChunkClass::Delta;
    }
    let Some((header_offset, unit_end)) = sps else {
        return ChunkClass::UndecodableKeyframe {
            fault: KeyframeFault::MissingSps,
        };
    };
    if !pps_before_idr {
        return ChunkClass::UndecodableKeyframe {
            fault: KeyframeFault::MissingPps,
        };
    }
    match codec_string(bytes, header_offset, unit_end) {
        Some(codec) => ChunkClass::Keyframe { codec },
        None => ChunkClass::UndecodableKeyframe {
            fault: KeyframeFault::SpsTooShort,
        },
    }
}

/// The `avc1.PPCCLL` codec string from the three bytes after the SPS NAL
/// header — profile_idc, constraint flags, level_idc — which is exactly what
/// `VideoDecoder.configure` expects to select a profile. All three bytes
/// must lie inside the SPS unit itself (`unit_end` is where the next start
/// code begins): a truncated SPS never borrows bytes from its neighbor.
fn codec_string(bytes: &[u8], sps_header_offset: usize, unit_end: usize) -> Option<String> {
    if sps_header_offset + 3 >= unit_end {
        return None;
    }
    let profile = bytes.get(sps_header_offset + 1)?;
    let constraint = bytes.get(sps_header_offset + 2)?;
    let level = bytes.get(sps_header_offset + 3)?;
    Some(format!("avc1.{profile:02x}{constraint:02x}{level:02x}"))
}

mod session;

pub use session::{ClaimAction, DecodeErrorRecovery, DecodeSession, FeedAction, SourceOwnership};

#[cfg(test)]
mod tests;
