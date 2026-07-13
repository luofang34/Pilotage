//! Canonical, byte-reproducible serialization and content digests.
//!
//! [`to_canonical`] emits the graph in a fixed order — nodes by id, attributes
//! by key, edges sorted, exceptions by id — with no timestamps and no trailing
//! whitespace, so the same graph always produces the same bytes. Re-parsing and
//! re-emitting the canonical text is a fixed point (`canonical == canonical of
//! parse of canonical`), which the tests pin. Digests are the SHA-256 of that
//! canonical text: a node's digest covers only its own block, so it is stable
//! against unrelated edits elsewhere in the graph.

use sha2::{Digest, Sha256};

use crate::graph::Graph;
use crate::node::Node;

/// The canonical-form version emitted by [`to_canonical`].
pub const CANONICAL_VERSION: u32 = 1;

/// Emits the canonical text form of the graph.
#[must_use]
pub fn to_canonical(graph: &Graph) -> String {
    let mut out = String::new();
    line(&mut out, &format!("evidence-graph {CANONICAL_VERSION}"));
    line(&mut out, &format!("scope {}", graph.scope.id));
    if !graph.scope.title.is_empty() {
        line(&mut out, &format!("scope-title {}", graph.scope.title));
    }
    for req in &graph.scope.requirements {
        line(&mut out, &format!("scope-requirement {req}"));
    }
    for node in graph.nodes() {
        out.push('\n');
        write_node(&mut out, node);
    }
    if graph.edges().next().is_some() {
        out.push('\n');
        for edge in graph.edges() {
            line(
                &mut out,
                &format!("edge {} {} {}", edge.from, edge.relation.token(), edge.to),
            );
        }
    }
    let mut exceptions: Vec<_> = graph.exceptions().iter().collect();
    exceptions.sort_by(|a, b| a.id.cmp(&b.id));
    for exception in exceptions {
        out.push('\n');
        line(&mut out, &format!("exception {}", exception.id));
        line(&mut out, &format!("covers {}", exception.covers));
        optional(&mut out, "owner", &exception.owner);
        optional(&mut out, "rationale", &exception.rationale);
        optional(&mut out, "status", &exception.status);
        optional(&mut out, "expiry", &exception.expiry);
        if let Some(review) = &exception.review {
            line(&mut out, &format!("review {review}"));
        }
    }
    out
}

/// The SHA-256 digest of a single node's canonical block.
#[must_use]
pub fn node_digest(node: &Node) -> [u8; 32] {
    let mut block = String::new();
    write_node(&mut block, node);
    Sha256::digest(block.as_bytes()).into()
}

/// The SHA-256 digest of the whole canonical graph text.
#[must_use]
pub fn graph_digest(graph: &Graph) -> [u8; 32] {
    Sha256::digest(to_canonical(graph).as_bytes()).into()
}

/// Writes one node block (no leading blank line).
fn write_node(out: &mut String, node: &Node) {
    line(out, &format!("node {} {}", node.id, node.kind.token()));
    if !node.title.is_empty() {
        line(out, &format!("title {}", node.title));
    }
    if let Some(locator) = &node.locator
        && !locator.is_empty()
    {
        line(out, &format!("locator {locator}"));
    }
    for (key, value) in &node.attrs {
        if value.is_empty() {
            line(out, &format!("attr {key}"));
        } else {
            line(out, &format!("attr {key} {value}"));
        }
    }
}

/// Pushes `keyword value` only when the value is non-empty.
fn optional(out: &mut String, keyword: &str, value: &str) {
    if !value.is_empty() {
        line(out, &format!("{keyword} {value}"));
    }
}

/// Pushes one line with trailing whitespace stripped.
fn line(out: &mut String, content: &str) {
    out.push_str(content.trim_end());
    out.push('\n');
}

#[cfg(test)]
mod tests;
