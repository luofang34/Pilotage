//! Panel admission harness (ADR-0029): foreign panels are admitted by
//! conformance, not trust.
//!
//! [`admit`] drives every registered panel through the shared canonical
//! corpus plus the panel's own extreme states, and through every
//! single-group withholding its descriptor declares, then checks five
//! families: the layer contract (well-formed scenes with every required
//! band present under every degradation), the background contract (the
//! declared capability is the band's actual behavior — `NotUsed` never
//! opens it, an owned band carries a full-frame opaque paint), budgets
//! (a text run outside the design frame is a counted warning), glyph
//! coverage (every text run resolves within the controlled
//! vocabulary), and honest status as a provenance rule (every numeric
//! run must claim the state group its value derives from, the claim is
//! bounded to the panel's required groups, and a run claiming the
//! withheld group may not be visible — wherever it is drawn). A shell
//! composes its registry and runs this once at integration time; a
//! panel that fails does not join an operational layout.
//!
//! Scope, honestly stated: this is a regression net for cooperative
//! panels, not a proof against adversarial ones. The numeral test is
//! ASCII digits, so letterform lookalikes (`O`, `I`, `S`) and digits
//! drawn as raw geometry are outside its sight; text extents are
//! conservative nominal-metric rectangles, not rendered ink; a
//! fabricator whose fake value is gated on the group it falsely claims
//! is, behaviourally, deriving from that group; and a claim is tested
//! against WITHHOLDING, not against the claimed group's resolved
//! status — a panel that fabricates only when data is present but
//! untrusted is outside the matrix's sight. Source-selection labels
//! are gated on stateful source-monitor state the harness never runs,
//! so their claims decode but are never exercised; and because the
//! harness draws the fixed empty configuration, a foreign panel whose
//! numeral legitimately derives from configuration is unadmittable by
//! construction, not merely untested.

mod admission;

pub use admission::{AdmissionError, AdmissionReport, AdmissionWarning, admit};
