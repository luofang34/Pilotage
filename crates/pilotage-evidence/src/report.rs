//! Human-readable rendering of gate and impact results.
//!
//! The rendered text is the CLI's product output. It always carries the
//! SIM / NOT FOR FLIGHT caveat and never describes the outcome as certification
//! or tool qualification. Findings suppressed by an exception are shown, loudly,
//! so a graph with recorded gaps can never read as a clean success.

use crate::gate::{GateReport, GateVerdict};
use crate::graph::Graph;
use crate::id::NodeId;
use crate::impact::ImpactReport;

/// Renders a gate report, listing every finding and every recorded exception.
#[must_use]
pub fn render_gate(report: &GateReport, graph: &Graph) -> String {
    let mut out = String::new();
    line(
        &mut out,
        &format!("=== Pilotage evidence gate — scope {} ===", report.scope_id),
    );
    line(
        &mut out,
        "SIM / NOT FOR FLIGHT engineering trace. Not certification, not tool qualification.",
    );
    line(
        &mut out,
        &format!("graph digest: {}", hex(&report.graph_digest)),
    );
    line(
        &mut out,
        &format!(
            "nodes: {}  edges: {}  exceptions: {}",
            report.node_count, report.edge_count, report.exception_count
        ),
    );
    line(&mut out, &format!("verdict: {}", verdict_line(report)));
    render_findings(&mut out, report);
    render_exceptions(&mut out, graph);
    out
}

/// Renders an impact report.
#[must_use]
pub fn render_impact(report: &ImpactReport) -> String {
    let mut out = String::new();
    line(
        &mut out,
        &format!("=== Change impact for {} ===", report.changed),
    );
    if !report.found {
        line(
            &mut out,
            &format!("warning: {} is not a node in the graph", report.changed),
        );
    }
    section(&mut out, "requirements", &report.requirements);
    section(&mut out, "coverage analyses", &report.analyses);
    section(
        &mut out,
        "verification cases/procedures/results",
        &report.tests,
    );
    section(&mut out, "reviews/approvals", &report.reviews);
    section(&mut out, "configuration bundles", &report.bundles);
    section(&mut out, "other", &report.other);
    out
}

fn render_findings(out: &mut String, report: &GateReport) {
    if report.findings.is_empty() {
        return;
    }
    let suppressed = report.findings.iter().filter(|f| f.excepted).count();
    let open = report.findings.len() - suppressed;
    out.push('\n');
    line(
        out,
        &format!("findings ({open} open, {suppressed} excepted):"),
    );
    for finding in &report.findings {
        let tag = if finding.excepted { "EXCEPTED " } else { "" };
        let subject = finding
            .subject
            .as_ref()
            .map_or_else(String::new, |s| format!(" {s}"));
        line(
            out,
            &format!(
                "  [{tag}{}]{subject}: {}",
                finding.code.label(),
                finding.detail
            ),
        );
    }
}

fn render_exceptions(out: &mut String, graph: &Graph) {
    if graph.exceptions().is_empty() {
        return;
    }
    out.push('\n');
    line(out, "recorded exceptions:");
    for exception in graph.exceptions() {
        let review = exception
            .review
            .as_ref()
            .map_or_else(|| "none".to_string(), ToString::to_string);
        line(
            out,
            &format!(
                "  {} covers {} — status {}, expiry {}, owner {}, review {review}",
                exception.id,
                exception.covers,
                blank_as(&exception.status, "MISSING"),
                blank_as(&exception.expiry, "MISSING"),
                blank_as(&exception.owner, "MISSING"),
            ),
        );
    }
}

fn verdict_line(report: &GateReport) -> String {
    match report.verdict {
        GateVerdict::Valid => "VALID".to_string(),
        GateVerdict::ValidWithExceptions => {
            let n = report.findings.iter().filter(|f| f.excepted).count();
            format!("VALID WITH EXCEPTIONS — {n} finding(s) suppressed by recorded exceptions")
        }
        GateVerdict::Invalid => {
            let n = report.findings.iter().filter(|f| !f.excepted).count();
            format!("INVALID — {n} open finding(s)")
        }
    }
}

fn section(out: &mut String, title: &str, ids: &[NodeId]) {
    if ids.is_empty() {
        return;
    }
    line(out, &format!("{title}:"));
    for id in ids {
        line(out, &format!("  {id}"));
    }
}

fn blank_as<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

fn line(out: &mut String, content: &str) {
    out.push_str(content);
    out.push('\n');
}

/// Lower-case hex of a 32-byte digest.
#[must_use]
pub fn hex(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
