//! The versioned, fixed-size, little-endian wire ABI for one synthetic-vision
//! contract frame, independent of any transport or renderer.
//!
//! The layout is versioned by a leading `u32` and has a fixed length
//! ([`SVS_FRAME_LEN`]); [`decode_frame`] fails closed on a truncated buffer, a
//! version it does not read, an enumerated field outside its known set, a
//! non-finite coordinate, or values that violate a semantic invariant (an MSL
//! height without a geoid, an invalid view). Encoding is allocation-free: it
//! writes into a fixed array.

use pilotage_frames::{ClockDomain, Epoch, FrameId, Quat, TimeScale};

use crate::availability::{AvailabilityReason, SvsAvailability};
use crate::datum::{
    GeodeticPosition, GeoidModelId, HorizontalDatum, LocalOriginId, VerticalDatum, VerticalPosition,
};
use crate::error::AbiError;
use crate::identity::{
    Accuracy, IntegrityLevel, SnapshotId, SourceIncarnation, SourceStamp, StatedAttitude,
    StatedPosition,
};
use crate::view::{
    CameraPose, MinificationPolicy, NearFarPolicy, OpticalConvention, ProjectionKind,
    ProjectionView, Viewport,
};

/// The ABI version. Bumped when the layout changes; decode refuses any other.
pub const ABI_VERSION: u32 = 1;

const STAMP_LEN: usize = 8 + 16 + 4 + 4 + (1 + 1 + 8) + 1 + (4 + 4) + 8;
const GEODETIC_LEN: usize = 8 + 8 + 1 + 8 + 1 + 2 + 8;
const POSITION_LEN: usize = GEODETIC_LEN + STAMP_LEN;
const QUAT_LEN: usize = 4 * 4;
const ATTITUDE_LEN: usize = QUAT_LEN + STAMP_LEN;
const VIEW_LEN: usize = (4 + 4) + (8 + 8) + 3 + (8 + 8) + (24 + QUAT_LEN + 1 + 1);

/// The fixed byte length of one encoded [`SvsFrame`].
pub const SVS_FRAME_LEN: usize = 4 + POSITION_LEN + ATTITUDE_LEN + VIEW_LEN + 2;

/// One complete synthetic-vision contract frame: a stated position, a stated
/// attitude, the projection view, and the availability verdict. TAWS is a
/// separate input and is deliberately not part of this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SvsFrame {
    /// The stated aircraft position (datum-explicit, stamped).
    pub position: StatedPosition,
    /// The stated aircraft attitude (stamped).
    pub attitude: StatedAttitude,
    /// The projection view a renderer must honor.
    pub view: ProjectionView,
    /// The availability verdict.
    pub availability: SvsAvailability,
}

// ---- little-endian writer / reader (bounds-checked, no panic) --------------

struct Writer<'a> {
    buf: &'a mut [u8],
    off: usize,
}

impl Writer<'_> {
    fn bytes(&mut self, b: &[u8]) {
        if let Some(slot) = self.buf.get_mut(self.off..self.off + b.len()) {
            slot.copy_from_slice(b);
        }
        self.off += b.len();
    }
    fn u8(&mut self, v: u8) {
        self.bytes(&[v]);
    }
    fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.bytes(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.bytes(&v.to_le_bytes());
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    off: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], AbiError> {
        let end = self.off + n;
        let slot = self.buf.get(self.off..end).ok_or(AbiError::Truncated {
            needed: end,
            got: self.buf.len(),
        })?;
        self.off = end;
        Ok(slot)
    }
    fn u8(&mut self) -> Result<u8, AbiError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, AbiError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, AbiError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64, AbiError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    fn f32(&mut self) -> Result<f32, AbiError> {
        let b = self.take(4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f64(&mut self) -> Result<f64, AbiError> {
        let b = self.take(8)?;
        Ok(f64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    fn finite_f64(&mut self, field: &'static str) -> Result<f64, AbiError> {
        let v = self.f64()?;
        if v.is_finite() {
            Ok(v)
        } else {
            Err(AbiError::NonFinite { field })
        }
    }
    /// Reads one enumerated byte and maps it, failing closed with the actual
    /// unknown value.
    fn enum_u8<T>(&mut self, field: &'static str, map: fn(u8) -> Option<T>) -> Result<T, AbiError> {
        let code = self.u8()?;
        map(code).ok_or(AbiError::UnknownEnum { field, value: code })
    }
}

const fn frame_from_u8(code: u8) -> Option<FrameId> {
    match FrameId::from_u8(code) {
        Ok(frame) => Some(frame),
        Err(_) => None,
    }
}

const fn clock_from_u8(code: u8) -> Option<ClockDomain> {
    match code {
        0 => Some(ClockDomain::VehicleBoot),
        1 => Some(ClockDomain::Simulation),
        2 => Some(ClockDomain::Gnss),
        3 => Some(ClockDomain::Ground),
        _ => None,
    }
}

const fn scale_from_u8(code: u8) -> Option<TimeScale> {
    match code {
        0 => Some(TimeScale::Monotonic),
        1 => Some(TimeScale::Gps),
        2 => Some(TimeScale::Tai),
        3 => Some(TimeScale::Utc),
        _ => None,
    }
}

// ---- encoders --------------------------------------------------------------

fn put_stamp(w: &mut Writer, s: &SourceStamp) {
    w.u64(s.source_id);
    w.bytes(&s.incarnation.0);
    w.u32(s.generation);
    w.u32(s.sequence);
    w.u8(s.acquired_at.clock as u8);
    w.u8(s.acquired_at.scale as u8);
    w.u64(s.acquired_at.nanos);
    w.u8(s.integrity.to_u8());
    w.u32(s.accuracy.horizontal_mm);
    w.u32(s.accuracy.vertical_mm);
    w.u64(s.snapshot.0);
}

fn put_quat(w: &mut Writer, q: &Quat) {
    w.f32(q.w);
    w.f32(q.x);
    w.f32(q.y);
    w.f32(q.z);
}

fn put_position(w: &mut Writer, p: &StatedPosition) {
    let g = &p.position;
    w.f64(g.latitude_deg);
    w.f64(g.longitude_deg);
    w.u8(g.horizontal_datum.to_u8());
    w.f64(g.vertical.height_m);
    w.u8(g.vertical.datum.to_u8());
    w.u16(g.vertical.geoid.0);
    w.u64(g.vertical.origin.0);
    put_stamp(w, &p.stamp);
}

fn put_view(w: &mut Writer, v: &ProjectionView) {
    w.u32(v.viewport.width_px);
    w.u32(v.viewport.height_px);
    w.f64(v.focal_x_px);
    w.f64(v.focal_y_px);
    w.u8(v.projection.to_u8());
    w.u8(v.minification.to_u8());
    w.u8(v.convention.to_u8());
    w.f64(v.near_far.near_m);
    w.f64(v.near_far.far_m);
    for c in v.camera.translation_m {
        w.f64(c);
    }
    put_quat(w, &v.camera.attitude);
    w.u8(v.camera.from_frame.to_u8());
    w.u8(v.camera.to_frame.to_u8());
}

fn put_availability(w: &mut Writer, a: SvsAvailability) {
    let (kind, reason) = match a {
        SvsAvailability::Available => (0, AvailabilityReason::Nominal),
        SvsAvailability::Degraded(r) => (1, r),
        SvsAvailability::Unavailable(r) => (2, r),
    };
    w.u8(kind);
    w.u8(reason.to_u8());
}

/// Serializes one frame into its fixed-size canonical byte form.
#[must_use]
pub fn encode_frame(frame: &SvsFrame) -> [u8; SVS_FRAME_LEN] {
    let mut buf = [0u8; SVS_FRAME_LEN];
    let mut w = Writer {
        buf: &mut buf,
        off: 0,
    };
    w.u32(ABI_VERSION);
    put_position(&mut w, &frame.position);
    put_quat(&mut w, &frame.attitude.attitude);
    put_stamp(&mut w, &frame.attitude.stamp);
    put_view(&mut w, &frame.view);
    put_availability(&mut w, frame.availability);
    buf
}

// ---- decoders --------------------------------------------------------------

fn get_stamp(r: &mut Reader) -> Result<SourceStamp, AbiError> {
    let source_id = r.u64()?;
    let inc = r.take(16)?;
    let mut incarnation = [0u8; 16];
    incarnation.copy_from_slice(inc);
    let generation = r.u32()?;
    let sequence = r.u32()?;
    let clock = r.enum_u8("clock", clock_from_u8)?;
    let scale = r.enum_u8("time_scale", scale_from_u8)?;
    let nanos = r.u64()?;
    let integrity = r.enum_u8("integrity", IntegrityLevel::from_u8)?;
    let horizontal_mm = r.u32()?;
    let vertical_mm = r.u32()?;
    let snapshot = SnapshotId(r.u64()?);
    Ok(SourceStamp {
        source_id,
        incarnation: SourceIncarnation(incarnation),
        generation,
        sequence,
        acquired_at: Epoch {
            clock,
            scale,
            nanos,
        },
        integrity,
        accuracy: Accuracy {
            horizontal_mm,
            vertical_mm,
        },
        snapshot,
    })
}

fn get_quat(r: &mut Reader) -> Result<Quat, AbiError> {
    Ok(Quat {
        w: r.f32()?,
        x: r.f32()?,
        y: r.f32()?,
        z: r.f32()?,
    })
}

fn get_position(r: &mut Reader) -> Result<StatedPosition, AbiError> {
    let latitude_deg = r.finite_f64("latitude_deg")?;
    let longitude_deg = r.finite_f64("longitude_deg")?;
    let hdatum = r.enum_u8("horizontal_datum", HorizontalDatum::from_u8)?;
    let height_m = r.finite_f64("height_m")?;
    let vdatum = r.enum_u8("vertical_datum", VerticalDatum::from_u8)?;
    let geoid = GeoidModelId(r.u16()?);
    let origin = LocalOriginId(r.u64()?);
    let stamp = get_stamp(r)?;
    let vertical = VerticalPosition::new(height_m, vdatum, geoid, origin)
        .map_err(|_| AbiError::Malformed { field: "vertical" })?;
    let position = GeodeticPosition::new(latitude_deg, longitude_deg, hdatum, vertical)
        .map_err(|_| AbiError::Malformed { field: "position" })?;
    Ok(StatedPosition { position, stamp })
}

fn get_view(r: &mut Reader) -> Result<ProjectionView, AbiError> {
    let width_px = r.u32()?;
    let height_px = r.u32()?;
    let focal_x_px = r.finite_f64("focal_x_px")?;
    let focal_y_px = r.finite_f64("focal_y_px")?;
    let projection = r.enum_u8("projection", ProjectionKind::from_u8)?;
    let minification = r.enum_u8("minification", MinificationPolicy::from_u8)?;
    let convention = r.enum_u8("convention", OpticalConvention::from_u8)?;
    let near_m = r.finite_f64("near_m")?;
    let far_m = r.finite_f64("far_m")?;
    let translation_m = [
        r.finite_f64("cam_x")?,
        r.finite_f64("cam_y")?,
        r.finite_f64("cam_z")?,
    ];
    let attitude = get_quat(r)?;
    let from_frame = r.enum_u8("camera_from_frame", frame_from_u8)?;
    let to_frame = r.enum_u8("camera_to_frame", frame_from_u8)?;
    let view = ProjectionView {
        viewport: Viewport {
            width_px,
            height_px,
        },
        focal_x_px,
        focal_y_px,
        projection,
        near_far: NearFarPolicy { near_m, far_m },
        minification,
        convention,
        camera: CameraPose {
            translation_m,
            attitude,
            from_frame,
            to_frame,
        },
    };
    view.validate()
        .map_err(|_| AbiError::Malformed { field: "view" })?;
    Ok(view)
}

fn get_availability(r: &mut Reader) -> Result<SvsAvailability, AbiError> {
    let kind = r.u8()?;
    let reason = r.enum_u8("availability_reason", AvailabilityReason::from_u8)?;
    match kind {
        0 => Ok(SvsAvailability::Available),
        1 => Ok(SvsAvailability::Degraded(reason)),
        2 => Ok(SvsAvailability::Unavailable(reason)),
        other => Err(AbiError::UnknownEnum {
            field: "availability_kind",
            value: other,
        }),
    }
}

/// Decodes one frame from its canonical byte form, failing closed.
///
/// # Errors
///
/// [`AbiError`] on truncation, an unsupported version, an unknown enumerated
/// value, a non-finite coordinate, or a semantically malformed block.
pub fn decode_frame(buf: &[u8]) -> Result<SvsFrame, AbiError> {
    let mut r = Reader { buf, off: 0 };
    let version = r.u32()?;
    if version != ABI_VERSION {
        return Err(AbiError::BadVersion { found: version });
    }
    let position = get_position(&mut r)?;
    let attitude = StatedAttitude {
        attitude: get_quat(&mut r)?,
        stamp: get_stamp(&mut r)?,
    };
    let view = get_view(&mut r)?;
    let availability = get_availability(&mut r)?;
    Ok(SvsFrame {
        position,
        attitude,
        view,
        availability,
    })
}

#[cfg(test)]
mod tests;
