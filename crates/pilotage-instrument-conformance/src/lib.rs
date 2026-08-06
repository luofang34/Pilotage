//! Panel admission harness (ADR-0029): foreign panels are admitted by
//! conformance, not trust.
//!
//! [`admit`] drives every registered panel through the shared canonical
//! corpus plus the panel's own extreme states, and through every
//! single-group withholding its descriptor declares, then checks four
//! families: the layer contract (well-formed scenes with every required
//! band present under every degradation), budgets (a text run outside
//! the design frame is a counted warning), glyph coverage (every text
//! run resolves within the controlled vocabulary), and honest status
//! (a withheld group's declared region may not show numeric text, and
//! the nothing-fed furniture itself must be numeral-free inside every
//! declared region). A shell composes its registry and runs this once
//! at integration time; a panel that fails does not join an
//! operational layout.
//!
//! Scope, honestly stated: this is a regression net for cooperative
//! panels, not a proof against adversarial ones. Group regions are
//! self-declared; the numeral test is ASCII digits, so letterform
//! lookalikes (`O`, `I`, `S`) and digits drawn as raw geometry are
//! outside its sight; and text extents are conservative nominal-metric
//! rectangles, not rendered ink.

mod admission;

pub use admission::{AdmissionError, AdmissionReport, AdmissionWarning, admit};
