//! The versioned, fixed-size, little-endian wire ABI for one synthetic-vision
//! contract frame, independent of any transport or renderer.
//!
//! The wire carries only *raw inputs*: the stated position and attitude, the
//! projection view, the producer-stated health of the inputs the contract
//! cannot itself check, and the frame reference time. It carries **no
//! availability**: availability is *derived* from validating those inputs, so a
//! wire producer can never self-report an `Available` scene over untrusted
//! inputs.
//!
//! [`decode_frame`] fails closed on a wrong-length buffer (a fixed block must
//! match exactly — trailing bytes are as suspect as truncation), a version it
//! does not read, an enumerated field outside its known set, a non-finite
//! coordinate, a non-unit aircraft attitude, or any semantic invariant
//! violation (an MSL height with no geoid, an incomplete calibration reference).
//! It returns a [`ValidatedSvsFrame`] whose availability is computed here, not
//! read. [`encode_frame`] serializes only a [`ValidatedSvsFrame`], so an invalid
//! frame cannot be encoded, and it is allocation-free.

use pilotage_frames::Epoch;

use crate::availability::{ExternalHealth, SvsAvailability, SvsInputs, derive_inputs};
use crate::error::{AbiError, GeoError};
use crate::identity::{StatedAttitude, StatedPosition};
use crate::view::ProjectionView;

mod codec;

/// The ABI version. Bumped when the layout changes; decode refuses any other.
pub const ABI_VERSION: u32 = 2;

/// Largest quaternion norm error tolerated before an attitude is refused as not
/// a rotation.
pub(crate) const ATTITUDE_NORM_TOLERANCE: f32 = 1e-4;

const STAMP_LEN: usize = 8 + 16 + 4 + 4 + (1 + 1 + 8) + 1 + (16 + 4 + 8);
const GEODETIC_LEN: usize = 8 + 8 + 1 + 2 + 8 + 1 + 2 + 4 + 4 + 8;
const POSITION_LEN: usize = GEODETIC_LEN + STAMP_LEN + (4 + 4);
const QUAT_LEN: usize = 4 * 4;
const ATTITUDE_LEN: usize = QUAT_LEN + STAMP_LEN + 4;
const VIEW_LEN: usize = 8 + 32 + 8 + 1 + 8 + 8 + 8 + 8 + 1;
const EXTERNAL_LEN: usize = 5;
const EPOCH_LEN: usize = 1 + 1 + 8;

/// The fixed byte length of one encoded frame.
pub const SVS_FRAME_LEN: usize =
    4 + POSITION_LEN + ATTITUDE_LEN + VIEW_LEN + EXTERNAL_LEN + EPOCH_LEN;

/// A raw, unvalidated synthetic-vision frame: the inputs a producer states,
/// before the contract derives availability. Build one and call
/// [`RawSvsFrame::validate`] (or round-trip through the wire) to obtain a
/// [`ValidatedSvsFrame`]; availability is never part of a raw frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawSvsFrame {
    /// The stated aircraft position (datum-explicit, stamped).
    pub position: StatedPosition,
    /// The stated aircraft attitude (stamped).
    pub attitude: StatedAttitude,
    /// The projection view (references the one validated calibration).
    pub view: ProjectionView,
    /// Producer-stated health of the inputs the contract cannot itself check.
    pub external: ExternalHealth,
    /// The producer's reference ("as of") time; position/attitude ages and the
    /// future-sample check are computed against it.
    pub reference_time: Epoch,
}

impl RawSvsFrame {
    /// Validates the raw inputs and derives availability, failing closed on any
    /// structural violation: a non-finite or out-of-range position, an
    /// incomplete datum identity, a non-unit aircraft attitude, or an invalid
    /// view. Availability (including time/coherence) is *derived*, never stated.
    ///
    /// # Errors
    ///
    /// A [`GeoError`] describing the first structural violation.
    pub fn validate(&self) -> Result<ValidatedSvsFrame, GeoError> {
        self.position.position.validate()?;
        if self
            .attitude
            .attitude
            .renormalized(ATTITUDE_NORM_TOLERANCE)
            .is_err()
        {
            return Err(GeoError::AttitudeNotARotation);
        }
        self.view.validate()?;
        Ok(ValidatedSvsFrame::assemble(
            self.position,
            self.attitude,
            self.view,
            self.external,
            self.reference_time,
        ))
    }
}

/// A validated synthetic-vision frame. Its availability and derived input health
/// are computed from the inputs and cannot be set independently: the fields are
/// private and the only ways to obtain one are [`RawSvsFrame::validate`] and
/// [`decode_frame`]. There is no path by which an untrusted input yields an
/// [`SvsAvailability::Available`] frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValidatedSvsFrame {
    position: StatedPosition,
    attitude: StatedAttitude,
    view: ProjectionView,
    external: ExternalHealth,
    reference_time: Epoch,
    inputs: SvsInputs,
    availability: SvsAvailability,
}

impl ValidatedSvsFrame {
    /// Derives the input health and availability and assembles the frame. The
    /// caller must have already validated the structural invariants.
    fn assemble(
        position: StatedPosition,
        attitude: StatedAttitude,
        view: ProjectionView,
        external: ExternalHealth,
        reference_time: Epoch,
    ) -> Self {
        let inputs = derive_inputs(&position, &attitude, &external, reference_time);
        let availability = SvsAvailability::assess(&inputs);
        Self {
            position,
            attitude,
            view,
            external,
            reference_time,
            inputs,
            availability,
        }
    }

    /// The stated aircraft position.
    #[must_use]
    pub fn position(&self) -> &StatedPosition {
        &self.position
    }
    /// The stated aircraft attitude.
    #[must_use]
    pub fn attitude(&self) -> &StatedAttitude {
        &self.attitude
    }
    /// The projection view.
    #[must_use]
    pub fn view(&self) -> &ProjectionView {
        &self.view
    }
    /// The producer-stated external input health.
    #[must_use]
    pub fn external_health(&self) -> ExternalHealth {
        self.external
    }
    /// The producer's reference time.
    #[must_use]
    pub fn reference_time(&self) -> Epoch {
        self.reference_time
    }
    /// The full derived input health.
    #[must_use]
    pub fn inputs(&self) -> SvsInputs {
        self.inputs
    }
    /// The derived availability verdict.
    #[must_use]
    pub fn availability(&self) -> SvsAvailability {
        self.availability
    }
}

/// Serializes one validated frame into its fixed-size canonical byte form. Only
/// a [`ValidatedSvsFrame`] can be encoded, so an invalid frame cannot be
/// serialized. Availability is not encoded — it is derived on decode.
#[must_use]
pub fn encode_frame(frame: &ValidatedSvsFrame) -> [u8; SVS_FRAME_LEN] {
    codec::encode(frame)
}

/// Decodes one frame from its canonical byte form, failing closed, and derives
/// availability from the decoded inputs.
///
/// # Errors
///
/// [`AbiError`] on a wrong-length buffer, an unsupported version, an unknown
/// enumerated value, a non-finite coordinate, a non-unit attitude, or a
/// semantically malformed block.
pub fn decode_frame(buf: &[u8]) -> Result<ValidatedSvsFrame, AbiError> {
    codec::decode(buf)
}

#[cfg(test)]
mod tests;
