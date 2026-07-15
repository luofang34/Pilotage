//! H.264 Annex-B access-unit classification: the byte-level half of the
//! `H264` FourCC video path (ADR-0016 codec dispatch), shared by every
//! consumer that must decide what a chunk MEANS before handing it to a
//! platform decoder — the browser's WebCodecs `VideoDecoder` today, a native
//! decoder on the same classification later. Decoding is never done here;
//! this module only reads NAL structure: keyframe determination, in-band
//! parameter-set presence, and the `avc1.PPCCLL` codec string derived from
//! the SPS.
//!
//! Fail closed: a keyframe that cannot configure a decoder (missing SPS,
//! missing PPS, or an SPS too short to name its profile) classifies as
//! undecodable with a typed reason. Input with no recognizable NAL units
//! yields no units and classifies as a delta frame, which a session layer
//! cannot act on before a decodable keyframe arrives — malformed bytes can
//! start nothing.

/// FourCC the wire tags an H.264 Annex-B video body with (ADR-0016).
pub const FOURCC: [u8; 4] = *b"H264";

// H.264 `nal_unit_type` values (the header byte after each start code,
// masked with 0x1F): IDR slice = keyframe, and the two parameter sets a
// decoder needs before it can be configured.
const NAL_IDR: u8 = 5;
const NAL_SPS: u8 = 7;
const NAL_PPS: u8 = 8;

/// One NAL unit located in an Annex-B buffer: its `nal_unit_type` and the
/// offset of its header byte (the byte after the start code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NalUnit {
    /// The unit's `nal_unit_type` (header byte & 0x1F).
    pub nal_type: u8,
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

/// Why a keyframe cannot configure a decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyframeFault {
    /// The access unit carries no SPS, so no profile/level is named.
    MissingSps,
    /// The access unit carries no PPS, so slice parameters are absent.
    MissingPps,
    /// The SPS ends before the three profile/constraint/level bytes.
    SpsTooShort,
}

impl KeyframeFault {
    /// A stable, human-readable reason for logs and typed failure surfaces.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::MissingSps => "missing in-band SPS",
            Self::MissingPps => "missing in-band PPS",
            Self::SpsTooShort => "SPS too short to name a profile",
        }
    }
}

/// What one access unit means to a decode session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkClass {
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

/// Classifies one access unit. The first SPS in stream order names the
/// codec, matching the parameter set a decoder would adopt.
#[must_use]
pub fn classify_chunk(bytes: &[u8]) -> ChunkClass {
    let mut idr = false;
    let mut sps: Option<usize> = None;
    let mut pps = false;
    for nal in nal_units(bytes) {
        match nal.nal_type {
            NAL_IDR => idr = true,
            NAL_SPS if sps.is_none() => sps = Some(nal.header_offset),
            NAL_PPS => pps = true,
            _ => {}
        }
    }
    if !idr {
        return ChunkClass::Delta;
    }
    let Some(header_offset) = sps else {
        return ChunkClass::UndecodableKeyframe {
            fault: KeyframeFault::MissingSps,
        };
    };
    if !pps {
        return ChunkClass::UndecodableKeyframe {
            fault: KeyframeFault::MissingPps,
        };
    }
    match codec_string(bytes, header_offset) {
        Some(codec) => ChunkClass::Keyframe { codec },
        None => ChunkClass::UndecodableKeyframe {
            fault: KeyframeFault::SpsTooShort,
        },
    }
}

/// The `avc1.PPCCLL` codec string from the three bytes after the SPS NAL
/// header — profile_idc, constraint flags, level_idc — which is exactly what
/// `VideoDecoder.configure` expects to select a profile. `None` when the
/// buffer ends before all three.
fn codec_string(bytes: &[u8], sps_header_offset: usize) -> Option<String> {
    let profile = bytes.get(sps_header_offset + 1)?;
    let constraint = bytes.get(sps_header_offset + 2)?;
    let level = bytes.get(sps_header_offset + 3)?;
    Some(format!("avc1.{profile:02x}{constraint:02x}{level:02x}"))
}

#[cfg(test)]
mod tests;
