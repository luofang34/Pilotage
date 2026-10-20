//! Mini 3 flight commands and independent FC reports.
//!
//! These types perform no I/O. A caller must bind them to one attachment,
//! enforce the write deadline at the transport, and never retry an uncertain
//! command. They do not advertise flight capabilities for other DJI models.

mod command;
mod report;
mod virtual_stick;

pub use command::{BasicFlightCommand, BasicFlightPacket};
pub(crate) use report::osd_payload;
pub use report::{Mini3FlightReport, VirtualStickReport};
pub use virtual_stick::{
    VirtualStickMode, VirtualStickModePacket, VirtualStickVelocity, VirtualStickVelocityError,
};

#[cfg(test)]
mod tests;
