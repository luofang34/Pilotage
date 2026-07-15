//! Standard-neutral lifecycle evidence graph and scoped no-orphan gate
//! (ASSURE-01).
//!
//! The graph is the maintained engineering record: typed [`Node`]s
//! (intended functions, hazards, requirements, design, implementation,
//! verification cases/procedures/results, coverage analyses, reviews,
//! approvals, configuration items, tools, anomalies, external evidence) joined
//! by typed [`Edge`]s ([`RelationKind`]). Every node carries a stable
//! [`NodeId`] and a content digest; the whole graph has a canonical,
//! byte-reproducible serialization ([`mod@canonical`]).
//!
//! Program views — DO-178C, ISO 26262, ECSS — are projections *over* this
//! record, held outside the core graph. Generated matrices and reports are
//! views, not the source of truth.
//!
//! The [`mod@gate`] enforces a *declared* [`Policy`](policy::Policy) over a
//! declared [`Scope`]: it does not assert the false rule that every file traces
//! to a certification objective. It fails closed — an empty or absent graph is
//! never reported valid — and surfaces justified [`Exception`]s without letting
//! them become a silent success. [`mod@impact`] answers "what does changing
//! this node affect", and [`mod@trace`] resolves the declared slice in both
//! directions — behavior down to a recorded result, and a result back up to
//! behavior and its configuration and tool identity.
//!
//! # SIM / NOT FOR FLIGHT
//!
//! This crate and any graph it validates are engineering tooling. A passing
//! gate establishes **no** DO-178C, ISO 26262, ECSS, ARP4754A/4761, TSO, ASIL,
//! or certification claim, and is not tool qualification (DO-330). It records
//! that the declared engineering trace is internally complete and resolvable,
//! nothing more.

#![forbid(unsafe_code)]

pub mod canonical;
pub mod gate;
pub mod impact;
pub mod parse;
pub mod policy;
pub mod report;
pub mod trace;

mod error;
mod graph;
mod id;
mod node;
mod relation;
mod scope;

#[cfg(test)]
mod testkit;

pub use error::EvidenceError;
pub use graph::Graph;
pub use id::{IdError, NodeId};
pub use node::{Node, NodeKind};
pub use relation::{Edge, RelationKind};
pub use scope::{Exception, Scope};
