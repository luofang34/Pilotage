//! Little-endian encode/decode for the SVS frame ABI, bounds-checked and
//! panic-free. This is the byte layer only; the frame types and the public
//! [`encode`]/[`decode`] entry points live in the parent module.

use pilotage_frames::{ClockDomain, Epoch, Quat, TimeScale};

use crate::availability::{ExternalHealth, InputHealth};
use crate::datum::{
    BaroSettingId, DatumRealizationId, GeodeticPosition, GeoidModelId, HorizontalDatum,
    LocalOriginId, TerrainRefId, VerticalDatum, VerticalPosition,
};
use crate::error::{AbiError, GeoError};
use crate::identity::{
    AttitudeQuality, CoherentSnapshot, IntegrityLevel, PositionQuality, SourceIncarnation,
    SourceStamp, StatedAttitude, StatedPosition,
};
use crate::view::{
    CalibrationId, CalibrationRef, MinificationPolicy, NearFarPolicy, Projection, ProjectionView,
};

use super::{ABI_VERSION, ATTITUDE_NORM_TOLERANCE, SVS_FRAME_LEN, ValidatedSvsFrame};

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
    fn epoch(&mut self, e: Epoch) {
        self.u8(e.clock as u8);
        self.u8(e.scale as u8);
        self.u64(e.nanos);
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    off: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], AbiError> {
        let end = self.off + n;
        let slot = self.buf.get(self.off..end).ok_or(AbiError::WrongLength {
            needed: SVS_FRAME_LEN,
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
    fn enum_u8<T>(&mut self, field: &'static str, map: fn(u8) -> Option<T>) -> Result<T, AbiError> {
        let code = self.u8()?;
        map(code).ok_or(AbiError::UnknownEnum { field, value: code })
    }
    fn health(&mut self, field: &'static str) -> Result<InputHealth, AbiError> {
        self.enum_u8(field, InputHealth::from_u8)
    }
    fn epoch(&mut self) -> Result<Epoch, AbiError> {
        let clock = self.enum_u8("clock", clock_from_u8)?;
        let scale = self.enum_u8("time_scale", scale_from_u8)?;
        let nanos = self.u64()?;
        Ok(Epoch {
            clock,
            scale,
            nanos,
        })
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
    w.epoch(s.acquired_at);
    w.u8(s.integrity.to_u8());
    w.bytes(&s.snapshot.producer.0);
    w.u32(s.snapshot.generation);
    w.u64(s.snapshot.id);
}

fn put_quat(w: &mut Writer, q: &Quat) {
    w.f32(q.w);
    w.f32(q.x);
    w.f32(q.y);
    w.f32(q.z);
}

fn put_geodetic(w: &mut Writer, g: &GeodeticPosition) {
    w.f64(g.latitude_deg);
    w.f64(g.longitude_deg);
    w.u8(g.horizontal_datum.to_u8());
    w.u16(g.realization.0);
    w.f64(g.vertical.height_m);
    w.u8(g.vertical.datum.to_u8());
    w.u16(g.vertical.geoid.0);
    w.u32(g.vertical.terrain_ref.0);
    w.u32(g.vertical.baro_setting.0);
    w.u64(g.vertical.origin.0);
}

fn put_position(w: &mut Writer, p: &StatedPosition) {
    put_geodetic(w, &p.position);
    put_stamp(w, &p.stamp);
    w.u32(p.quality.horizontal_mm);
    w.u32(p.quality.vertical_mm);
}

fn put_view(w: &mut Writer, v: &ProjectionView) {
    w.u32(v.calibration.calibration_id.0);
    w.bytes(&v.calibration.content_hash);
    w.u8(v.projection.kind_u8());
    let (ex, ey) = match v.projection {
        Projection::Perspective => (0.0, 0.0),
        Projection::Orthographic {
            extent_x_m,
            extent_y_m,
        } => (extent_x_m, extent_y_m),
    };
    w.f64(ex);
    w.f64(ey);
    w.f64(v.near_far.near_m);
    w.f64(v.near_far.far_m);
    w.u8(v.minification.to_u8());
}

fn put_external(w: &mut Writer, e: &ExternalHealth) {
    w.u8(e.integrity.to_u8());
    w.u8(e.calibration.to_u8());
    w.u8(e.database.to_u8());
    w.u8(e.coverage.to_u8());
    w.u8(e.renderer.to_u8());
}

/// Serializes one validated frame into its fixed-size canonical byte form.
pub(super) fn encode(frame: &ValidatedSvsFrame) -> [u8; SVS_FRAME_LEN] {
    let mut buf = [0u8; SVS_FRAME_LEN];
    let mut w = Writer {
        buf: &mut buf,
        off: 0,
    };
    w.u32(ABI_VERSION);
    put_position(&mut w, frame.position());
    put_quat(&mut w, &frame.attitude().attitude);
    put_stamp(&mut w, &frame.attitude().stamp);
    w.u32(frame.attitude().quality.angular_mrad);
    put_view(&mut w, frame.view());
    put_external(&mut w, &frame.external_health());
    w.epoch(frame.reference_time());
    buf
}

// ---- decoders --------------------------------------------------------------

fn get_stamp(r: &mut Reader) -> Result<SourceStamp, AbiError> {
    let source_id = r.u64()?;
    let mut incarnation = [0u8; 16];
    incarnation.copy_from_slice(r.take(16)?);
    let generation = r.u32()?;
    let sequence = r.u32()?;
    let acquired_at = r.epoch()?;
    let integrity = r.enum_u8("integrity", IntegrityLevel::from_u8)?;
    let mut producer = [0u8; 16];
    producer.copy_from_slice(r.take(16)?);
    let snap_generation = r.u32()?;
    let snap_id = r.u64()?;
    Ok(SourceStamp {
        source_id,
        incarnation: SourceIncarnation(incarnation),
        generation,
        sequence,
        acquired_at,
        integrity,
        snapshot: CoherentSnapshot {
            producer: SourceIncarnation(producer),
            generation: snap_generation,
            id: snap_id,
        },
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

fn get_geodetic(r: &mut Reader) -> Result<GeodeticPosition, AbiError> {
    let latitude_deg = r.finite_f64("latitude_deg")?;
    let longitude_deg = r.finite_f64("longitude_deg")?;
    let hdatum = r.enum_u8("horizontal_datum", HorizontalDatum::from_u8)?;
    let realization = DatumRealizationId(r.u16()?);
    let height_m = r.finite_f64("height_m")?;
    let vdatum = r.enum_u8("vertical_datum", VerticalDatum::from_u8)?;
    let geoid = GeoidModelId(r.u16()?);
    let terrain_ref = TerrainRefId(r.u32()?);
    let baro_setting = BaroSettingId(r.u32()?);
    let origin = LocalOriginId(r.u64()?);
    let vertical =
        VerticalPosition::new(height_m, vdatum, geoid, terrain_ref, baro_setting, origin).map_err(
            |reason| AbiError::Malformed {
                field: "vertical",
                reason,
            },
        )?;
    GeodeticPosition::new(latitude_deg, longitude_deg, hdatum, realization, vertical).map_err(
        |reason| AbiError::Malformed {
            field: "position",
            reason,
        },
    )
}

fn get_position(r: &mut Reader) -> Result<StatedPosition, AbiError> {
    let position = get_geodetic(r)?;
    let stamp = get_stamp(r)?;
    let quality = PositionQuality {
        horizontal_mm: r.u32()?,
        vertical_mm: r.u32()?,
    };
    Ok(StatedPosition {
        position,
        stamp,
        quality,
    })
}

fn get_attitude(r: &mut Reader) -> Result<StatedAttitude, AbiError> {
    let attitude = get_quat(r)?;
    if attitude.renormalized(ATTITUDE_NORM_TOLERANCE).is_err() {
        return Err(AbiError::Malformed {
            field: "attitude",
            reason: GeoError::AttitudeNotARotation,
        });
    }
    let stamp = get_stamp(r)?;
    let quality = AttitudeQuality {
        angular_mrad: r.u32()?,
    };
    Ok(StatedAttitude {
        attitude,
        stamp,
        quality,
    })
}

fn get_view(r: &mut Reader) -> Result<ProjectionView, AbiError> {
    let calibration_id = CalibrationId(r.u32()?);
    let mut content_hash = [0u8; 32];
    content_hash.copy_from_slice(r.take(32)?);
    let projection_kind = r.u8()?;
    let extent_x_m = r.f64()?;
    let extent_y_m = r.f64()?;
    let near_m = r.finite_f64("near_m")?;
    let far_m = r.finite_f64("far_m")?;
    let minification = r.enum_u8("minification", MinificationPolicy::from_u8)?;
    let projection = match projection_kind {
        0 => Projection::Perspective,
        1 => Projection::Orthographic {
            extent_x_m,
            extent_y_m,
        },
        other => {
            return Err(AbiError::UnknownEnum {
                field: "projection",
                value: other,
            });
        }
    };
    let view = ProjectionView {
        calibration: CalibrationRef {
            calibration_id,
            content_hash,
        },
        projection,
        near_far: NearFarPolicy { near_m, far_m },
        minification,
    };
    view.validate().map_err(|reason| AbiError::Malformed {
        field: "view",
        reason,
    })?;
    Ok(view)
}

fn get_external(r: &mut Reader) -> Result<ExternalHealth, AbiError> {
    Ok(ExternalHealth {
        integrity: r.health("external_integrity")?,
        calibration: r.health("external_calibration")?,
        database: r.health("external_database")?,
        coverage: r.health("external_coverage")?,
        renderer: r.health("external_renderer")?,
    })
}

/// Decodes one frame from its canonical byte form, failing closed.
pub(super) fn decode(buf: &[u8]) -> Result<ValidatedSvsFrame, AbiError> {
    if buf.len() != SVS_FRAME_LEN {
        return Err(AbiError::WrongLength {
            needed: SVS_FRAME_LEN,
            got: buf.len(),
        });
    }
    let mut r = Reader { buf, off: 0 };
    let version = r.u32()?;
    if version != ABI_VERSION {
        return Err(AbiError::BadVersion { found: version });
    }
    let position = get_position(&mut r)?;
    let attitude = get_attitude(&mut r)?;
    let view = get_view(&mut r)?;
    let external = get_external(&mut r)?;
    let reference_time = r.epoch()?;
    Ok(ValidatedSvsFrame::assemble(
        position,
        attitude,
        view,
        external,
        reference_time,
    ))
}
