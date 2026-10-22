use crate::{DumlFrame, RouteAddress};
use turret_dji::{frame_checksum, header_checksum};

/// An aircraft-native operation on the Mini 3 basic flight endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BasicFlightCommand {
    /// Request automatic takeoff. Motor start is not takeoff completion.
    Takeoff,
    /// Request automatic landing. This is not an emergency motor stop.
    Land,
    /// Request return to home. The aircraft must establish its own prerequisites.
    ReturnHome,
    /// Cancel native landing and request hover; feedback must confirm the change.
    CancelLanding,
    /// Continue landing only when fresh FC feedback requests confirmation.
    ConfirmLanding,
}

impl BasicFlightCommand {
    /// Encode one APP-to-FC request with an attachment-local sequence.
    ///
    /// The caller must not reuse a sequence for another live transaction.
    #[must_use]
    pub fn packet(self, sequence: u16) -> BasicFlightPacket {
        let value = match self {
            Self::Takeoff => 1,
            Self::Land => 2,
            Self::ReturnHome => 6,
            // DJI OSDK FlightCommand::exitLanding uses the same function endpoint.
            // https://github.com/dji-sdk/Onboard-SDK/blob/master/osdk-core/api/inc/dji_control.hpp
            Self::CancelLanding => 14,
            Self::ConfirmLanding => 30,
        };
        let [low, high] = sequence.to_le_bytes();
        let mut bytes = [0x55, 14, 4, 0, 2, 3, low, high, 0x40, 3, 0x2a, value, 0, 0];
        bytes[3] = header_checksum(&bytes[..3]);
        let checksum = frame_checksum(&bytes[..12]).to_le_bytes();
        bytes[12..].copy_from_slice(&checksum);
        BasicFlightPacket { bytes, sequence }
    }
}

/// One fixed-format DUML request, without USB transport framing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BasicFlightPacket {
    bytes: [u8; 14],
    sequence: u16,
}

impl BasicFlightPacket {
    /// The complete checksummed DUML packet.
    #[must_use]
    pub const fn bytes(&self) -> &[u8; 14] {
        &self.bytes
    }

    /// The request sequence used to correlate its response.
    #[must_use]
    pub const fn sequence(&self) -> u16 {
        self.sequence
    }

    /// The result code from the matching FC response.
    ///
    /// Call this only after a complete write on the same attachment. A zero
    /// result acknowledges the command; it does not confirm aircraft motion.
    #[must_use]
    pub fn response(self, frame: &DumlFrame) -> Option<u8> {
        if frame.version() != 1
            || frame.sender() != RouteAddress(3)
            || frame.receiver() != RouteAddress(2)
            || frame.flags() != 0x80
            || frame.command_set() != 3
            || frame.command_id() != 0x2a
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
