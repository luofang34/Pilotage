//! The `VIDEO_FRAME_V2` body codec: the one definition of the per-frame
//! capture-identity header, shared by the host encoder and the browser
//! decoder so the two can never drift (ADR-0020).
//!
//! A v2 video stream is a single host-initiated uni stream whose first byte is
//! the kind tag (owned by the driver) followed by this body: a fixed
//! little-endian capture-identity header, a 4-byte codec FourCC, a `u32` length
//! prefix, and the encoded payload. [`OFFSET`] is the single source of truth
//! for every field position; both [`encode_v2`] and [`decode_v2`] read it, and
//! the host's frame builder delegates its encode here.

use crate::wire::MeasurementClock;

/// Byte offsets of each field in the fixed v2 capture-identity header, in wire
/// order. The header ends at [`OFFSET.codec`](Offsets::codec); the FourCC,
/// length prefix, and payload follow.
pub struct Offsets {
    /// Routing source id (0 = onboard FPV, 1 = chase).
    pub source_id: usize,
    /// Source boot/attachment generation (`u32`).
    pub source_epoch: usize,
    /// Opaque 128-bit attachment/boot identity (16 bytes).
    pub source_incarnation: usize,
    /// Wrapping per-source group sequence (`u32`).
    pub sequence: usize,
    /// Sim capture time in the declared clock domain (`u64`).
    pub capture_time: usize,
    /// Capture clock code (see [`clock_code`]).
    pub capture_clock: usize,
    /// 1 when a clock mapping is present, 0 when unavailable.
    pub mapping_available: usize,
    /// Target clock code of the mapping, or 0 when unavailable.
    pub mapping_target_clock: usize,
    /// Signed offset applied to the capture time by the mapping (`i64`).
    pub mapping_offset: usize,
    /// Quantified mapping error bound (`u64`).
    pub clock_error_bound: usize,
    /// Host receive time (`u64`).
    pub receive_time: usize,
    /// Host publication time (`u64`).
    pub publication_time: usize,
    /// Camera identity (`u32`).
    pub camera_id: usize,
    /// Calibration identity (`u32`), 0 = none.
    pub calibration_id: usize,
    /// Start of the 4-byte codec FourCC (also the header length).
    pub codec: usize,
    /// Start of the `u32` little-endian payload length prefix.
    pub length: usize,
    /// Start of the encoded payload.
    pub payload: usize,
}

/// Field offsets of the v2 header. Keep this the only place field positions are
/// spelled; a reordered field here re-points both encoder and decoder at once.
pub const OFFSET: Offsets = Offsets {
    source_id: 0,
    source_epoch: 1,
    source_incarnation: 5,
    sequence: 21,
    capture_time: 25,
    capture_clock: 33,
    mapping_available: 34,
    mapping_target_clock: 35,
    mapping_offset: 36,
    clock_error_bound: 44,
    receive_time: 52,
    publication_time: 60,
    camera_id: 68,
    calibration_id: 72,
    codec: 76,
    length: 80,
    payload: 84,
};

/// Length of the fixed capture-identity header, before the FourCC and length
/// prefix. Equal to [`OFFSET.codec`](Offsets::codec).
pub const META_LEN: usize = OFFSET.codec;

/// The v2 header's clock codes match the `pilotage.v1` [`MeasurementClock`]
/// enum: 1 vehicle-boot, 2 simulation. 0 (unspecified) is reserved for an
/// absent target clock; the encoder never emits it for a real capture clock.
const CLOCK_VEHICLE_BOOT: u8 = MeasurementClock::VehicleBoot as u8;
const CLOCK_SIMULATION: u8 = MeasurementClock::Simulation as u8;
const CLOCK_ABSENT: u8 = MeasurementClock::Unspecified as u8;

/// Wire code for a [`MeasurementClock`], for a clock that is actually present
/// (never [`MeasurementClock::Unspecified`]).
#[must_use]
pub fn clock_code(clock: MeasurementClock) -> u8 {
    clock as u8
}

/// The capture-identity header, decoded into plain fields. Encodes to and
/// decodes from exactly [`META_LEN`] bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureHeader {
    /// Routing source id (0 = onboard FPV, 1 = chase).
    pub source_id: u8,
    /// Source boot/attachment generation.
    pub source_epoch: u32,
    /// Opaque 128-bit attachment/boot identity.
    pub source_incarnation: [u8; 16],
    /// Wrapping per-source group sequence.
    pub sequence: u32,
    /// Sim capture time in the declared clock domain, nanoseconds.
    pub capture_time_ns: u64,
    /// Capture clock code.
    pub capture_clock: u8,
    /// Whether a clock mapping to the flight-state clock is present.
    pub mapping_available: bool,
    /// Target clock code of the mapping (0 when unavailable).
    pub mapping_target_clock: u8,
    /// Signed offset the mapping applies to the capture time, nanoseconds.
    pub mapping_offset_ns: i64,
    /// Quantified mapping error bound, nanoseconds.
    pub clock_error_bound_ns: u64,
    /// Host receive time, nanoseconds.
    pub receive_time_ns: u64,
    /// Host publication time, nanoseconds.
    pub publication_time_ns: u64,
    /// Camera identity.
    pub camera_id: u32,
    /// Calibration identity (0 = none).
    pub calibration_id: u32,
}

/// A decoded v2 video body: its header, codec FourCC, and payload slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedFrame<'a> {
    /// The capture-identity header.
    pub header: CaptureHeader,
    /// The codec FourCC (e.g. `MJPG`, `H264`).
    pub codec: [u8; 4],
    /// The encoded payload, exactly the length the prefix declared.
    pub payload: &'a [u8],
}

/// Why a v2 video body failed to decode structurally (as opposed to failing
/// the semantic [`CaptureHeader::contract_fault`] check).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// Fewer bytes than the fixed header plus FourCC and length prefix.
    #[error("v2 body {len} bytes is shorter than the {min}-byte header+prefix")]
    TooShort {
        /// Bytes actually present.
        len: usize,
        /// Minimum bytes a well-formed body needs before its payload.
        min: usize,
    },
    /// The declared payload length does not match the bytes that follow.
    #[error("v2 body declares {declared} payload bytes but {actual} follow")]
    LengthMismatch {
        /// The `u32` length prefix's value.
        declared: usize,
        /// Bytes actually present after the prefix.
        actual: usize,
    },
    /// The `mapping_available` octet was neither the canonical `0` nor `1`. A
    /// non-canonical flag is refused before it is normalized to a `bool`, so a
    /// value such as `2` can never be silently read as "unavailable".
    #[error("v2 body mapping_available octet {value} is not canonical 0 or 1")]
    NonCanonicalMappingFlag {
        /// The out-of-range octet the wire carried.
        value: u8,
    },
}

/// A capture header field that violates the encoder's contract, for a typed,
/// fail-closed rejection reason mirroring the browser's `metaFault`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContractFault {
    /// The JS-facing field name that failed (camelCase, as the viewer logs it).
    pub field: &'static str,
    /// The rule it violated.
    pub rule: &'static str,
}

impl CaptureHeader {
    /// The first field to violate the encoder's contract, or `None` when the
    /// header is admissible. Fail-closed: an unknown capture clock, or a
    /// mapping whose target clock is inconsistent with its availability flag,
    /// is rejected. A mapping declared unavailable legitimately carries target
    /// clock 0 (the absent-clock sentinel), so that case is accepted: the
    /// target clock is bound to the availability flag, never checked alone.
    #[must_use]
    pub fn contract_fault(&self) -> Option<ContractFault> {
        if !is_present_clock(self.capture_clock) {
            return Some(ContractFault {
                field: "captureClock",
                rule: "malformed",
            });
        }
        let target_ok = if self.mapping_available {
            is_present_clock(self.mapping_target_clock)
        } else {
            self.mapping_target_clock == CLOCK_ABSENT
        };
        if !target_ok {
            return Some(ContractFault {
                field: "mappingTargetClock",
                rule: "malformed",
            });
        }
        None
    }
}

fn is_present_clock(code: u8) -> bool {
    code == CLOCK_VEHICLE_BOOT || code == CLOCK_SIMULATION
}

/// Serializes a v2 body: the fixed capture-identity header, the codec FourCC, a
/// little-endian `u32` length prefix, and the payload. Returns `None` if the
/// payload is larger than `u32::MAX` (a length the prefix cannot express).
#[must_use]
pub fn encode_v2(header: &CaptureHeader, codec: [u8; 4], payload: &[u8]) -> Option<Vec<u8>> {
    let len = u32::try_from(payload.len()).ok()?;
    let mut out = Vec::with_capacity(OFFSET.payload + payload.len());
    out.push(header.source_id);
    out.extend_from_slice(&header.source_epoch.to_le_bytes());
    out.extend_from_slice(&header.source_incarnation);
    out.extend_from_slice(&header.sequence.to_le_bytes());
    out.extend_from_slice(&header.capture_time_ns.to_le_bytes());
    out.push(header.capture_clock);
    out.push(u8::from(header.mapping_available));
    out.push(header.mapping_target_clock);
    out.extend_from_slice(&header.mapping_offset_ns.to_le_bytes());
    out.extend_from_slice(&header.clock_error_bound_ns.to_le_bytes());
    out.extend_from_slice(&header.receive_time_ns.to_le_bytes());
    out.extend_from_slice(&header.publication_time_ns.to_le_bytes());
    out.extend_from_slice(&header.camera_id.to_le_bytes());
    out.extend_from_slice(&header.calibration_id.to_le_bytes());
    out.extend_from_slice(&codec);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

/// Parses a v2 body (the bytes after the kind tag) into its header, FourCC, and
/// payload. Reads field positions from [`OFFSET`], so it can never disagree
/// with [`encode_v2`] about the layout.
///
/// # Errors
///
/// [`DecodeError::TooShort`] if the body is shorter than the header plus FourCC
/// and length prefix; [`DecodeError::LengthMismatch`] if the declared payload
/// length does not match the trailing bytes exactly.
pub fn decode_v2(body: &[u8]) -> Result<DecodedFrame<'_>, DecodeError> {
    if body.len() < OFFSET.payload {
        return Err(DecodeError::TooShort {
            len: body.len(),
            min: OFFSET.payload,
        });
    }
    let declared = u32::from_le_bytes([
        body[OFFSET.length],
        body[OFFSET.length + 1],
        body[OFFSET.length + 2],
        body[OFFSET.length + 3],
    ]);
    let declared = declared as usize;
    let payload = &body[OFFSET.payload..];
    if payload.len() != declared {
        return Err(DecodeError::LengthMismatch {
            declared,
            actual: payload.len(),
        });
    }
    // Reject a non-canonical mapping-availability flag before normalizing it to
    // a bool: `== 1` alone would read 2 (or any other octet) as "unavailable",
    // which — paired with target clock 0 — would slip past the contract check.
    let mapping_octet = body[OFFSET.mapping_available];
    if mapping_octet > 1 {
        return Err(DecodeError::NonCanonicalMappingFlag {
            value: mapping_octet,
        });
    }
    let mut incarnation = [0_u8; 16];
    incarnation.copy_from_slice(&body[OFFSET.source_incarnation..OFFSET.sequence]);
    let header = CaptureHeader {
        source_id: body[OFFSET.source_id],
        source_epoch: le_u32(body, OFFSET.source_epoch),
        source_incarnation: incarnation,
        sequence: le_u32(body, OFFSET.sequence),
        capture_time_ns: le_u64(body, OFFSET.capture_time),
        capture_clock: body[OFFSET.capture_clock],
        mapping_available: mapping_octet == 1,
        mapping_target_clock: body[OFFSET.mapping_target_clock],
        mapping_offset_ns: le_i64(body, OFFSET.mapping_offset),
        clock_error_bound_ns: le_u64(body, OFFSET.clock_error_bound),
        receive_time_ns: le_u64(body, OFFSET.receive_time),
        publication_time_ns: le_u64(body, OFFSET.publication_time),
        camera_id: le_u32(body, OFFSET.camera_id),
        calibration_id: le_u32(body, OFFSET.calibration_id),
    };
    let codec = [
        body[OFFSET.codec],
        body[OFFSET.codec + 1],
        body[OFFSET.codec + 2],
        body[OFFSET.codec + 3],
    ];
    Ok(DecodedFrame {
        header,
        codec,
        payload,
    })
}

fn le_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn le_u64(bytes: &[u8], at: usize) -> u64 {
    let mut buf = [0_u8; 8];
    buf.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(buf)
}

fn le_i64(bytes: &[u8], at: usize) -> i64 {
    let mut buf = [0_u8; 8];
    buf.copy_from_slice(&bytes[at..at + 8]);
    i64::from_le_bytes(buf)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        CLOCK_SIMULATION, CLOCK_VEHICLE_BOOT, CaptureHeader, DecodeError, META_LEN, OFFSET,
        decode_v2, encode_v2,
    };

    fn bounded_header() -> CaptureHeader {
        CaptureHeader {
            source_id: 1,
            source_epoch: 7,
            source_incarnation: [0xAB; 16],
            sequence: 42,
            capture_time_ns: 123_456,
            capture_clock: CLOCK_SIMULATION,
            mapping_available: true,
            mapping_target_clock: CLOCK_VEHICLE_BOOT,
            mapping_offset_ns: -1_000,
            clock_error_bound_ns: 250,
            receive_time_ns: 5_000,
            publication_time_ns: 6_000,
            camera_id: 9,
            calibration_id: 3,
        }
    }

    #[test]
    fn offsets_and_meta_len_are_pinned() {
        // A drift in any field position or the header length is a wire break.
        assert_eq!(META_LEN, 76);
        assert_eq!(OFFSET.length, 80);
        assert_eq!(OFFSET.payload, 84);
    }

    #[test]
    fn encode_decode_round_trips() {
        let header = bounded_header();
        let jpeg = vec![0xFF, 0xD8, 1, 2, 3, 0xFF, 0xD9];
        let body = encode_v2(&header, *b"MJPG", &jpeg).expect("frames");
        let decoded = decode_v2(&body).expect("decodes");
        assert_eq!(decoded.header, header);
        assert_eq!(&decoded.codec, b"MJPG");
        assert_eq!(decoded.payload, jpeg.as_slice());
    }

    #[test]
    fn decode_reads_pinned_offsets() {
        let body = encode_v2(&bounded_header(), *b"MJPG", &[1, 2, 3]).expect("frames");
        assert_eq!(body[OFFSET.source_id], 1);
        assert_eq!(&body[OFFSET.codec..OFFSET.codec + 4], b"MJPG");
        assert_eq!(body[OFFSET.capture_clock], CLOCK_SIMULATION);
        assert_eq!(body[OFFSET.mapping_target_clock], CLOCK_VEHICLE_BOOT);
    }

    #[test]
    fn unavailable_mapping_with_zero_target_is_admissible() {
        // An unavailable capture-clock mapping carries target clock 0 (the
        // absent-clock sentinel); that is legitimate, not malformed. Aviate
        // publishes exactly this shape, so a validator requiring a known
        // clock unconditionally would drop every one of its frames.
        let header = CaptureHeader {
            mapping_available: false,
            mapping_target_clock: 0,
            mapping_offset_ns: 0,
            clock_error_bound_ns: 0,
            ..bounded_header()
        };
        assert_eq!(header.contract_fault(), None);
    }

    #[test]
    fn available_mapping_with_zero_target_is_rejected() {
        let header = CaptureHeader {
            mapping_available: true,
            mapping_target_clock: 0,
            ..bounded_header()
        };
        assert_eq!(
            header.contract_fault().expect("fault").field,
            "mappingTargetClock"
        );
    }

    #[test]
    fn unknown_capture_clock_is_rejected() {
        let header = CaptureHeader {
            capture_clock: 7,
            ..bounded_header()
        };
        assert_eq!(
            header.contract_fault().expect("fault").field,
            "captureClock"
        );
    }

    #[test]
    fn non_canonical_mapping_flag_is_rejected_before_normalization() {
        // A hostile octet of 2 would normalize to `false` under `== 1`; paired
        // with target clock 0 it would then pass the contract check. Decoding
        // must refuse it outright rather than read it as "unavailable".
        let mut body = encode_v2(&bounded_header(), *b"MJPG", &[1, 2, 3]).expect("frames");
        body[OFFSET.mapping_available] = 2;
        assert!(matches!(
            decode_v2(&body),
            Err(DecodeError::NonCanonicalMappingFlag { value: 2 })
        ));
        // The canonical values still decode.
        body[OFFSET.mapping_available] = 0;
        assert!(decode_v2(&body).is_ok());
        body[OFFSET.mapping_available] = 1;
        assert!(decode_v2(&body).is_ok());
    }

    #[test]
    fn short_body_and_length_mismatch_are_errors() {
        assert!(matches!(
            decode_v2(&[0_u8; 10]),
            Err(DecodeError::TooShort { .. })
        ));
        let mut body = encode_v2(&bounded_header(), *b"MJPG", &[1, 2, 3]).expect("frames");
        body.pop();
        assert!(matches!(
            decode_v2(&body),
            Err(DecodeError::LengthMismatch { .. })
        ));
    }
}
