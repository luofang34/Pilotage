//! Transport-independent geospatial, projection, coherence, and availability
//! contract for synthetic vision and conformal HUD (SVS-01).
//!
//! This crate is the typed contract that a synthetic-vision system and a
//! head-up display both consume; it is **not** a synthetic-vision
//! implementation, a renderer, or a terrain database, and completion defines a
//! simulator/engineering contract, **not** SVS/SVGS approval. SIM / NOT FOR
//! FLIGHT.
//!
//! Everything here fails closed. Datum, units, reference, and clock domain are
//! explicit at the type level and can never be silently inferred: there is no
//! bare altitude, no untagged position, no implicit frame or clock. A missing,
//! inconsistent, or untrusted input resolves to [`SvsAvailability::Degraded`] or
//! [`SvsAvailability::Unavailable`] with a finite, traceable
//! [`AvailabilityReason`] — never a plausible-looking normal scene. The wire
//! ABI ([`abi`]) is versioned, fixed-size, little-endian, and decode fails
//! closed on any unknown value.
//!
//! # Consumed vocabularies
//!
//! Reference frames, clock domains, time scales, epochs, and rotations come
//! from `pilotage-frames` ([`pilotage_frames::FrameId`],
//! [`pilotage_frames::Epoch`], [`pilotage_frames::Quat`]); source identity
//! reuses the AV-01 `MeasurementStamp` shape ([`identity`] documents the
//! mapping). The vertical-datum vocabulary parallels instrument-state's
//! `AltitudeClass` with a documented mapping ([`datum`]) rather than a
//! dependency, because this crate is foundational and instrument-state is a
//! consumer.
//!
//! # TAWS independence
//!
//! A terrain-awareness alert is an independent [`taws::TawsAlert`] input. No
//! type or path here lets synthetic vision become an implicit TAWS: the SVS
//! availability verdict and a TAWS alert are separate values with separate
//! failure behavior.

#![no_std]
#![forbid(unsafe_code)]

#[cfg(test)]
extern crate std;

mod abi;
mod availability;
mod datum;
mod error;
mod identity;
mod taws;
mod view;

pub use abi::{
    ABI_VERSION, RawSvsFrame, SVS_FRAME_LEN, ValidatedSvsFrame, decode_frame, encode_frame,
};
pub use availability::{
    AvailabilityReason, ExternalHealth, InputHealth, MAX_FRESH_AGE_NS, MAX_USABLE_AGE_NS,
    SvsAvailability, SvsInputs, derive_inputs, health_from_integrity,
};
pub use datum::{
    BaroSettingId, DatumRealizationId, GeoTile, GeodeticPosition, GeoidModelId, HorizontalDatum,
    LocalOriginId, TerrainRefId, VerticalDatum, VerticalPosition, wrap_longitude_deg,
};
pub use error::{AbiError, AgeError, GeoError};
pub use identity::{
    AttitudeQuality, CoherentSnapshot, IntegrityLevel, PositionQuality, SourceIncarnation,
    SourceStamp, StatedAttitude, StatedPosition,
};
pub use taws::{TawsAlert, TawsHazard};
pub use view::{CalibrationRef, MinificationPolicy, NearFarPolicy, Projection, ProjectionView};
