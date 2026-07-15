//! Filesystem resolution of verification-case selectors.
//!
//! A verification case names a real test by `locator` (a repo-relative path) and
//! a `test` attribute (the test function symbol). Resolution reads the file and
//! confirms the symbol is defined, so a case cannot claim a test that has been
//! renamed or deleted. A missing file or symbol is a fail-closed finding, never
//! a silent pass. Presence of the selector fields is checked separately, so a
//! case missing them is skipped here to avoid double-reporting.

use std::fs;
use std::path::Path;

use crate::gate::{Finding, FindingCode};
use crate::graph::Graph;
use crate::node::NodeKind;
use crate::policy::Policy;

/// Resolves every verification-case selector against `repo_root`.
pub(super) fn resolve(
    graph: &Graph,
    policy: &Policy,
    repo_root: &Path,
    findings: &mut Vec<Finding>,
) {
    for id in graph.ids_of_kind(NodeKind::VerificationCase) {
        let node = match graph.node(id) {
            Some(node) => node,
            None => continue,
        };
        let path = match node.locator.as_deref().filter(|p| !p.is_empty()) {
            Some(path) => path,
            None => continue,
        };
        let test = match node.attr(&policy.selector_attr) {
            Some(test) => test,
            None => continue,
        };
        let full = match crate::gate::contained::resolve_contained(repo_root, path) {
            Ok(full) => full,
            Err(escape) => {
                findings.push(Finding::new(
                    FindingCode::UnresolvedSelector,
                    Some(id.clone()),
                    format!("selector for {id}: file {}", escape.detail(path)),
                ));
                continue;
            }
        };
        match fs::read_to_string(&full) {
            Err(_) => findings.push(Finding::new(
                FindingCode::UnresolvedSelector,
                Some(id.clone()),
                format!("selector for {id}: file {path} not found under repo root"),
            )),
            Ok(text) if !defines(&text, test) => findings.push(Finding::new(
                FindingCode::UnresolvedSelector,
                Some(id.clone()),
                format!("selector for {id}: test {test} not defined in {path}"),
            )),
            Ok(_) => {}
        }
    }
}

/// Whether `text` defines a `fn <symbol>(`.
fn defines(text: &str, symbol: &str) -> bool {
    text.contains(&format!("fn {symbol}("))
}
