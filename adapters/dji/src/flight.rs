//! Mini 3 flight commands and independent FC reports.
//!
//! These types perform no I/O. A caller must bind them to one attachment,
//! enforce the write deadline at the transport, and never retry an uncertain
//! command. They do not advertise flight capabilities for other DJI models.

mod command;
mod report;
mod stream;
mod virtual_stick;

pub use command::{BasicFlightCommand, BasicFlightPacket};
pub(crate) use report::osd_payload;
pub use report::{Mini3FlightReport, VirtualStickReport};
pub use stream::{
    VirtualStickBinding, VirtualStickStream, VirtualStickStreamError, VirtualStickStreamPhase,
    VirtualStickWrite,
};
pub use virtual_stick::{
    HORIZONTAL_LIMIT_MPS, VERTICAL_LIMIT_MPS, VirtualStickMode, VirtualStickModePacket,
    VirtualStickVelocity, VirtualStickVelocityError, YAW_RATE_LIMIT_RPS,
};

#[cfg(test)]
mod tests;
