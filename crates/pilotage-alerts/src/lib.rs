//! Deterministic central flightcrew alert manager and aural contract
//! (ALR-01).
//!
//! Alert meaning belongs in one place, not scattered across widget colors
//! and flags. This crate is that place: a pure, bounded, `no_std`,
//! allocation-free, I/O-free state machine that turns typed fault
//! conditions into a priority-ordered visual alert list and a single aural
//! command. It generates alerts; it does not render them. Panels consume
//! [`AlertOutput`] and decide layout, never severity or inhibition.
//!
//! The core guarantee is determinism: [`AlertManager::step`] reads no
//! interior clock and holds no hidden state, so the same
//! `(policy, events, context, time)` applied to the same manager state
//! produces byte-identical output and ordering. Time and flight/mode
//! context are caller inputs.
//!
//! Priority follows AC 25.1322-1 concepts: warning outranks caution
//! outranks advisory, with status and maintenance below and silent; ties
//! break by ascending stable [`AlertId`]. Warnings and cautions latch until
//! acknowledged or cleared; lower classes self-clear. Acknowledgement
//! silences the aural but never clears an active condition. Capacity is
//! bounded and overflow is fail-visible: a full table drops the
//! lowest-priority alert, counted with a wrapping counter, and never
//! silently displaces a higher-priority one. Completion demonstrates no
//! regulatory compliance.

#![no_std]

#[cfg(test)]
extern crate std;

mod class;
mod condition;
mod event;
mod manager;
mod output;
mod profile;

pub use class::{AlertClass, AlertState, AuralToken};
pub use condition::{
    AlertCondition, AlertId, AltFault, DisplayFault, DynFault, MiscompareFault, NavFault,
    SystemNote, class_of,
};
pub use event::{AlertContext, AlertEvent, FlightPhase};
pub use manager::AlertManager;
pub use output::{ActiveAlert, AlertOutput, MAX_ACTIVE_ALERTS, ManagerHealth};
pub use profile::{AlertProfile, InhibitRule, MAX_INHIBIT_RULES, ProfileError};
