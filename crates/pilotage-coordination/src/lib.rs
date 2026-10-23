//! The coordinator role (ADR-0052, ADR-0028). A team lane speaks the same
//! mission intents, receipts and status to its operator as a vehicle's lane
//! does, and works only through its members' lanes: it decomposes a survey
//! objective into one band per member, compiles a team proposal from the
//! members' own proposals, passes an approval down as member approvals
//! inside the team's envelope, and adapts within that envelope: a line a
//! member flew without usable pictures is flown again without a new
//! operator approval, while the envelope allows it.
//!
//! A member can itself be a team, so hierarchies use one contract. Sans-IO:
//! the host carries out the member intents the lane returns.

mod allocation;
mod geometry;
mod team;

#[cfg(test)]
mod tests;

pub use allocation::{Assignment, allocate};
pub use geometry::Local;
pub use team::{Member, MemberIntent, TeamConfig, TeamLane, TeamReply};
