//! Protocol-free source identity and the ingress admission gate (ADR-0018).
//!
//! ADR-0018 defines how a receiver decides whether a measurement group may
//! replace display state: a group is admitted only when its incarnation is
//! authorized and its epoch and sequence advance under wrap-safe serial
//! arithmetic. Duplicates, reordering, already-seen incarnations, older epochs,
//! and acquisition-time regressions are counted and cannot replace state or
//! refresh its age.
//!
//! Those rules are stated here in types that carry no wire dependency, because
//! they are needed in compositions where no wire exists. ADR-0037's local-source
//! path — a tablet talking straight to a panel — still has source
//! identity: the panel boots, reconnects, and can be swapped. Expressing
//! identity only in protocol types would make the rules unavailable exactly
//! where there is no protocol.
//!
//! The crate is `no_std` and allocation-free so it can sit under the instrument
//! family (ADR-0017), whose leaf crates it is intended to feed.

#![no_std]

mod admission;
mod source;
mod stamp;

pub use admission::{Admission, RejectReason, SourceGate};
pub use source::{MeasurementClock, SourceIncarnation, SourceIntegrity, SourceRole};
pub use stamp::MeasurementStamp;
