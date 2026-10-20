use crate::{DumlFrame, RouteAddress};
use turret_dji::{frame_checksum, header_checksum};

/// An explicit request to change the FC's virtual-stick mode.
///
/// Acceptance does not prove exclusive control or a device-side input timeout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtualStickMode {
    /// Request virtual-stick mode without changing the aircraft configuration.
    Enable,
    /// Request release of virtual-stick mode. This does not stop the motors.
    Disable,
}

impl VirtualStickMode {
    /// Encode one request. The caller owns attachment and sequence fencing.
    #[must_use]
    pub fn packet(self, sequence: u16) -> VirtualStickModePacket {
        let [low, high] = sequence.to_le_bytes();
        let value = match self {
            Self::Enable => 1,
            Self::Disable => 2,
        };
        let mut bytes = [0x55, 14, 4, 0, 2, 3, low, high, 0x40, 3, 0x80, value, 0, 0];
        seal(&mut bytes);
        VirtualStickModePacket { bytes, sequence }
    }
}

/// One mode request without USB framing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualStickModePacket {
    bytes: [u8; 14],
    sequence: u16,
}

impl VirtualStickModePacket {
    /// Return the complete checksummed packet.
    #[must_use]
    pub const fn bytes(&self) -> &[u8; 14] {
        &self.bytes
    }

    /// Read the first decision from a matching FC response.
    ///
    /// The caller must first confirm a full write on the same attachment.
    #[must_use]
    pub fn response(self, frame: &DumlFrame) -> Option<u8> {
        if frame.version() != 1
            || frame.sender() != RouteAddress(3)
            || frame.receiver() != RouteAddress(2)
            || frame.flags() != 0x80
            || frame.command_set() != 3
            || frame.command_id() != 0x80
            || frame.sequence() != self.sequence
        {
            return None;
        }
        let [code] = frame.payload() else {
            return None;
        };
        Some(*code)
    }
}

/// A bounded BODY-FRD velocity value for offline encoding.
///
/// This type does not enable a mode or advertise a flight capability. Its
/// limits are an experimental host policy, not the aircraft's flight envelope.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualStickVelocity {
    values: [f32; 4],
}

/// Invalid velocity input for the experimental encoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VirtualStickVelocityError {
    /// Every component must be finite and inside the experiment's limits.
    #[error("virtual-stick input exceeds the finite experimental envelope")]
    OutsideEnvelope,
}

impl VirtualStickVelocity {
    /// Validate forward/right/down speed in m/s and clockwise yaw rate in rad/s.
    ///
    /// Horizontal speed is limited to 1 m/s in total, vertical speed to
    /// 0.5 m/s, and yaw rate to 30 degrees/s.
    ///
    /// # Errors
    /// Returns an error for a non-finite value or a value outside these limits.
    pub fn new(
        forward: f32,
        right: f32,
        down: f32,
        yaw_rate: f32,
    ) -> Result<Self, VirtualStickVelocityError> {
        let values = [forward, right, down, yaw_rate];
        if values.iter().any(|value| !value.is_finite())
            || forward.hypot(right) > 1.0
            || down.abs() > 0.5
            || yaw_rate.abs() > core::f32::consts::FRAC_PI_6
        {
            return Err(VirtualStickVelocityError::OutsideEnvelope);
        }
        Ok(Self { values })
    }

    /// Encode BODY velocity with upward vertical speed and yaw rate in degrees/s.
    ///
    /// The FC sub-endpoint does not acknowledge each value. No I/O is performed.
    #[must_use]
    pub fn packet(self, sequence: u16) -> [u8; 30] {
        let [forward, right, down, yaw_rate] = self.values;
        let [low, high] = sequence.to_le_bytes();
        let mut bytes = [0; 30];
        bytes[..12].copy_from_slice(&[0x55, 30, 4, 0, 2, 0xc3, low, high, 0, 3, 0x8e, 0x4b]);
        for (target, value) in bytes[12..28].as_chunks_mut::<4>().0.iter_mut().zip([
            forward,
            right,
            -down,
            yaw_rate.to_degrees(),
        ]) {
            target.copy_from_slice(&value.to_le_bytes());
        }
        seal(&mut bytes);
        bytes
    }
}

fn seal(bytes: &mut [u8]) {
    bytes[3] = header_checksum(&bytes[..3]);
    let end = bytes.len() - 2;
    let crc = frame_checksum(&bytes[..end]).to_le_bytes();
    bytes[end..].copy_from_slice(&crc);
}

#[cfg(test)]
mod tests;
