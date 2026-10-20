use crate::{DumlFrame, RouteAddress};

/// The independently reported motor and flight state of a Mini 3.
///
/// `is_flying == false` does not establish ground contact. A running motor
/// does not establish an armed-state contract or completed takeoff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mini3FlightReport {
    /// DUML sequence of the source report.
    pub sequence: u16,
    /// The FC's motor-operation flag.
    pub motors_running: bool,
    /// The FC's flight-state flag.
    pub is_flying: bool,
    /// The raw FC mode, without a generic Pilotage mode translation.
    pub mode: u8,
}

impl Mini3FlightReport {
    /// Read the fixed Mini 3 OSD layout on its observed relay route.
    ///
    /// A valid report still needs attachment identity, sequence continuity,
    /// and age checks before it can authorize a command.
    #[must_use]
    pub fn from_push(frame: &DumlFrame) -> Option<Self> {
        let payload = osd_payload(frame)?;
        (payload[48] == 87).then_some(Self {
            sequence: frame.sequence(),
            motors_running: payload[32] & 8 != 0,
            is_flying: payload[32] & 0x0e == 0x0c,
            mode: payload[30] & 0x7f,
        })
    }

    /// Whether the expected OSD route explicitly identifies another model.
    ///
    /// Negative identity evidence must invalidate the existing binding even
    /// when the replacement device restarts its sequence.
    #[must_use]
    pub fn reports_other_model(frame: &DumlFrame) -> bool {
        osd_payload(frame).is_some_and(|payload| payload[48] != 87)
    }
}

/// The FC's virtual-stick enable flag, separate from control ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualStickReport {
    /// DUML sequence of the source report.
    pub sequence: u16,
    /// Whether the FC reports virtual-stick mode enabled.
    pub enabled: bool,
    /// Whether multiple flight modes are enabled in the aircraft configuration.
    pub multiple_flight_modes: bool,
    /// Whether the aircraft reports novice mode enabled.
    pub novice_mode: bool,
}

impl VirtualStickReport {
    /// Read the low-frequency report used by the Mini 3 profile.
    ///
    /// This message does not identify the aircraft model or the controlling
    /// client. The caller must establish both bindings separately.
    #[must_use]
    pub fn from_push(frame: &DumlFrame) -> Option<Self> {
        if !push_route(frame, 0x44) {
            return None;
        }
        let payload: &[u8; 78] = frame.payload().try_into().ok()?;
        let configuration = u16::from_le_bytes([payload[20], payload[21]]);
        Some(Self {
            sequence: frame.sequence(),
            enabled: payload[32] & 2 != 0,
            multiple_flight_modes: configuration & (1 << 6) != 0,
            novice_mode: configuration & (1 << 11) != 0,
        })
    }
}

pub(crate) fn osd_payload(frame: &DumlFrame) -> Option<&[u8; 70]> {
    push_route(frame, 0x43)
        .then(|| frame.payload().try_into().ok())
        .flatten()
}

fn push_route(frame: &DumlFrame, command: u8) -> bool {
    frame.version() == 1
        && frame.sender() == RouteAddress(14)
        && frame.receiver() == RouteAddress(2)
        && frame.flags() == 0
        && frame.command_set() == 3
        && frame.command_id() == command
}
