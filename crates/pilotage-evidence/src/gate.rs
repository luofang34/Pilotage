//! The scoped no-orphan gate.
//!
//! [`validate`] holds the declared in-scope assurance case to the declared
//! [`Policy`]: every in-scope requirement must trace both
//! upstream (to a hazard and an intended function, or a recorded rationale) and
//! downstream (design, implementation, verification case, verification result);
//! every recorded result must resolve to its executed case, a configuration
//! baseline, a tool identity, and a covered requirement; every derived
//! requirement must carry its safety impact, rationale, disposition, and
//! review. The gate fails closed: an empty or absent graph is never valid, and
//! a justified [`Exception`](crate::scope::Exception) is always surfaced rather
//! than silently turned into success.

mod checks;
mod selector;

use std::path::Path;

use crate::graph::Graph;
use crate::id::NodeId;
use crate::policy::Policy;

/// Why a single finding was raised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindingCode {
    /// The graph, or the declared scope, is empty (fail-closed).
    EmptyGraph,
    /// A scope requirement is absent or is not a requirement node.
    MissingScopeRequirement,
    /// An edge names a node that does not exist.
    DanglingEdge,
    /// A requirement does not reach the required upstream roots.
    MissingUpstream,
    /// A requirement does not reach a required downstream layer.
    MissingDownstreamLayer,
    /// A result does not resolve to case, baseline, tool, and requirement.
    ResultUnresolved,
    /// A verification case that covers an in-scope requirement has no recorded
    /// result (no verification-result resolves to it).
    MissingResult,
    /// A recorded result lacks the immutable provenance that makes it evidence
    /// (executed command, configuration digest, tool version, artifact).
    PlaceholderResult,
    /// A review exists but is pending, incomplete, or not independent, so it
    /// cannot yield a VALID verdict.
    ReviewIncomplete,
    /// A derived requirement is missing safety impact, rationale, disposition,
    /// or review.
    DerivedRequirementIncomplete,
    /// A verification case's test selector is missing or does not resolve.
    UnresolvedSelector,
    /// An exception cannot apply because it is itself incomplete.
    ExceptionMalformed,
}

impl FindingCode {
    /// A short stable label for reports.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            FindingCode::EmptyGraph => "empty-graph",
            FindingCode::MissingScopeRequirement => "missing-scope-requirement",
            FindingCode::DanglingEdge => "dangling-edge",
            FindingCode::MissingUpstream => "missing-upstream",
            FindingCode::MissingDownstreamLayer => "missing-downstream-layer",
            FindingCode::ResultUnresolved => "result-unresolved",
            FindingCode::MissingResult => "missing-result",
            FindingCode::PlaceholderResult => "placeholder-result",
            FindingCode::ReviewIncomplete => "review-incomplete",
            FindingCode::DerivedRequirementIncomplete => "derived-requirement-incomplete",
            FindingCode::UnresolvedSelector => "unresolved-selector",
            FindingCode::ExceptionMalformed => "exception-malformed",
        }
    }
}

/// One gate finding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// The reason.
    pub code: FindingCode,
    /// The node the finding concerns, when it names one.
    pub subject: Option<NodeId>,
    /// A human-readable explanation.
    pub detail: String,
    /// Whether a well-formed exception covers this finding.
    pub excepted: bool,
}

impl Finding {
    fn new(code: FindingCode, subject: Option<NodeId>, detail: impl Into<String>) -> Self {
        Self {
            code,
            subject,
            detail: detail.into(),
            excepted: false,
        }
    }
}

/// The overall gate outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateVerdict {
    /// No findings.
    Valid,
    /// Findings exist but all are covered by well-formed exceptions.
    ValidWithExceptions,
    /// At least one uncovered finding, or the graph is empty.
    Invalid,
}

impl GateVerdict {
    /// Whether the gate passes (a clean exit code).
    #[must_use]
    pub fn passed(self) -> bool {
        !matches!(self, GateVerdict::Invalid)
    }
}

/// The result of running the gate.
#[derive(Clone, Debug)]
pub struct GateReport {
    /// The declared scope id.
    pub scope_id: String,
    /// The canonical graph digest at validation time.
    pub graph_digest: [u8; 32],
    /// Node count.
    pub node_count: usize,
    /// Edge count.
    pub edge_count: usize,
    /// Exception count.
    pub exception_count: usize,
    /// All findings, excepted ones included.
    pub findings: Vec<Finding>,
    /// The computed verdict.
    pub verdict: GateVerdict,
}

/// Validates the graph against the policy without touching the filesystem.
///
/// Verification-case selectors are checked for presence only; use
/// [`validate_resolving`] to also confirm each selector resolves to a real
/// test on disk.
#[must_use]
pub fn validate(graph: &Graph, policy: &Policy) -> GateReport {
    finalize(graph, policy, collect(graph, policy, None))
}

/// Validates the graph and additionally resolves every verification-case
/// selector against `repo_root`, failing closed on any that does not resolve.
#[must_use]
pub fn validate_resolving(graph: &Graph, policy: &Policy, repo_root: &Path) -> GateReport {
    finalize(graph, policy, collect(graph, policy, Some(repo_root)))
}

/// Runs the checks, short-circuiting to a single fail-closed finding when the
/// graph or the declared scope is empty.
fn collect(graph: &Graph, policy: &Policy, repo_root: Option<&Path>) -> Vec<Finding> {
    if graph.is_empty() || graph.scope.is_empty() {
        return vec![Finding::new(
            FindingCode::EmptyGraph,
            None,
            "graph has no nodes or the declared scope names no requirement",
        )];
    }
    let mut findings = Vec::new();
    checks::scope_roots(graph, &mut findings);
    checks::dangling_edges(graph, &mut findings);
    checks::requirement_traces(graph, policy, &mut findings);
    checks::results(graph, &mut findings);
    checks::result_provenance(graph, policy, &mut findings);
    checks::cases_have_results(graph, &mut findings);
    checks::reviews_complete(graph, policy, &mut findings);
    checks::derived_requirements(graph, policy, &mut findings);
    checks::selectors_present(graph, policy, &mut findings);
    if let Some(root) = repo_root {
        selector::resolve(graph, policy, root, &mut findings);
    }
    findings
}

/// Applies exceptions and computes the verdict.
fn finalize(graph: &Graph, policy: &Policy, mut findings: Vec<Finding>) -> GateReport {
    apply_exceptions(graph, policy, &mut findings);
    let verdict = verdict_of(graph, &findings);
    GateReport {
        scope_id: graph.scope.id.clone(),
        graph_digest: crate::canonical::graph_digest(graph),
        node_count: graph.nodes().count(),
        edge_count: graph.edges().count(),
        exception_count: graph.exceptions().len(),
        findings,
        verdict,
    }
}

/// Marks findings covered by well-formed exceptions and appends a finding for
/// every exception too incomplete to apply.
fn apply_exceptions(graph: &Graph, policy: &Policy, findings: &mut Vec<Finding>) {
    let mut malformed = Vec::new();
    for exception in graph.exceptions() {
        let problems = exception_problems(graph, policy, exception);
        if problems.is_empty() {
            for finding in findings.iter_mut() {
                if finding.subject.as_ref() == Some(&exception.covers) {
                    finding.excepted = true;
                }
            }
        } else {
            malformed.push(Finding::new(
                FindingCode::ExceptionMalformed,
                None,
                format!("exception {}: {}", exception.id, problems.join("; ")),
            ));
        }
    }
    findings.extend(malformed);
}

/// The reasons an exception cannot apply, if any.
fn exception_problems(
    graph: &Graph,
    policy: &Policy,
    exception: &crate::scope::Exception,
) -> Vec<String> {
    let mut problems: Vec<String> = exception
        .missing_fields()
        .into_iter()
        .map(|field| format!("missing {field}"))
        .collect();
    if policy.exception_requires_review {
        match &exception.review {
            None => problems.push("missing independent review".to_string()),
            Some(review) if !graph.contains(review) => {
                problems.push(format!("review {review} is not a node"));
            }
            Some(_) => {}
        }
    }
    if let Some(as_of) = policy.exception_as_of.as_deref()
        && exception.is_expired(as_of)
    {
        problems.push(format!(
            "expired {} (as of {as_of})",
            exception.expiry.trim()
        ));
    }
    problems
}

fn verdict_of(graph: &Graph, findings: &[Finding]) -> GateVerdict {
    if graph.is_empty() {
        return GateVerdict::Invalid;
    }
    if findings.iter().any(|f| !f.excepted) {
        GateVerdict::Invalid
    } else if findings.is_empty() {
        GateVerdict::Valid
    } else {
        GateVerdict::ValidWithExceptions
    }
}

#[cfg(test)]
mod tests;
