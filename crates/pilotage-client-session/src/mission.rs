//! The client side of a host's mission lane (ADR-0052), shared by every
//! client that steers one: the operator's App, the command-line operator,
//! and automated agents. Each sends the same [`MissionCommand`]s, resolves
//! them into intents with the same [`ApprovalPolicy`], and follows the lane
//! with the same [`MissionDesk`], so a person and an agent steer a lane
//! through one path.
//!
//! The commands and the desk's [`MissionView`] have a JSON form, which an
//! agent reads and writes one line at a time. The host decides what each
//! client may do: only a client admitted with the approver secret may
//! approve.

mod approval;
mod command;
mod desk;
mod survey_file;
mod view;

pub use approval::{ApprovalPolicy, approval, approved_height_m};
pub use command::{IntentKind, MissionCommand};
pub use desk::{DeskRefusal, FiledReceipt, MissionDesk, Offered};
pub use survey_file::SurveyFile;
pub use view::{MemberView, MissionView, ProposalView, ReceiptView, TaskView, hex};

#[cfg(test)]
mod tests;
