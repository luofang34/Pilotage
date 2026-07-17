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
            Ok(text) if !defines_in(path, &text, test) => findings.push(Finding::new(
                FindingCode::UnresolvedSelector,
                Some(id.clone()),
                format!("selector for {id}: test {test} not defined in {path}"),
            )),
            Ok(_) => {}
        }
    }
}

/// Whether `text` defines `symbol` as a genuine test, dispatching on the
/// locator's language. A `.mjs`/`.js` test file uses the JavaScript rule
/// (definition plus explicit runner registration); everything else uses
/// the Rust rule (`#[test]`-attributed `fn`). The browser suites are real
/// CI gates, so a case may bind one of their tests with the same
/// rename/delete safety the Rust rule gives.
pub(super) fn defines_in(path: &str, text: &str, symbol: &str) -> bool {
    if path.ends_with(".mjs") || path.ends_with(".js") {
        defines_js(text, symbol)
    } else {
        defines(text, symbol)
    }
}

/// Whether `text` defines `symbol` as a genuine JavaScript test: a
/// top-level `function <symbol>(` definition AND a separate bare
/// reference to `<symbol>` — the runner registration these suites use
/// (`for (const test of [ ...names ])`). Requiring both mirrors the Rust
/// rule's "definition plus `#[test]`": a helper that is defined but never
/// registered does not resolve, and a name that appears only in the
/// runner list without a definition does not either. Comment and
/// string-literal occurrences are blanked first, so no decoy resolves.
pub(super) fn defines_js(text: &str, symbol: &str) -> bool {
    let needle = format!("function {symbol}(");
    let mut in_block_comment = false;
    let mut defined = false;
    let mut registered = false;
    for raw in text.lines() {
        let code = code_of_line(raw, &mut in_block_comment);
        if !defined && code.contains(&needle) {
            defined = true;
            continue;
        }
        if !registered && mentions_symbol(&code, symbol) {
            registered = true;
        }
    }
    defined && registered
}

/// Whether `code` contains `symbol` as a whole identifier (both
/// neighbours are non-identifier characters), so a longer name that
/// merely contains `symbol` as a substring is not a match.
fn mentions_symbol(code: &str, symbol: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = code[from..].find(symbol) {
        let pos = from + rel;
        let before_ok = code[..pos]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        let after = &code[pos + symbol.len()..];
        let after_ok = after
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        if before_ok && after_ok {
            return true;
        }
        from = pos + symbol.len();
    }
    false
}

/// Whether `text` defines `symbol` as a genuine test: an actual function
/// definition — not a mention inside a comment or string literal — whose
/// preceding attribute lines include a `#[test]`-family attribute. A selector
/// can therefore never resolve against a commented-out test, a name embedded
/// in a string, or a plain helper function the harness would not run.
pub(super) fn defines(text: &str, symbol: &str) -> bool {
    let needle = format!("fn {symbol}(");
    let mut in_block_comment = false;
    let mut code_lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        let code = code_of_line(raw, &mut in_block_comment);
        let is_definition = code.find(&needle).is_some_and(|pos| {
            let at_boundary = code[..pos]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
            at_boundary && fn_qualifiers_only(code[..pos].trim())
        });
        if is_definition && preceded_by_test_attribute(&code_lines) {
            return true;
        }
        code_lines.push(code);
    }
    false
}

/// The code content of one line: string-literal bodies blanked and `//` and
/// `/* */` comments removed, tracking block-comment state across lines.
fn code_of_line(raw: &str, in_block_comment: &mut bool) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if *in_block_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                *in_block_comment = false;
            }
            continue;
        }
        if in_string {
            if c == '\\' {
                chars.next();
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '/' if chars.peek() == Some(&'/') => break,
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                *in_block_comment = true;
            }
            _ => out.push(c),
        }
    }
    out
}

/// Whether the text before `fn` on a definition line is only function
/// qualifiers (visibility, `async`, `const`, `unsafe`, an extern ABI).
fn fn_qualifiers_only(prefix: &str) -> bool {
    prefix.split_whitespace().all(|token| {
        matches!(token, "pub" | "async" | "const" | "unsafe" | "extern")
            || token.starts_with("pub(")
            || token.starts_with('"')
    })
}

/// Whether the nearest preceding non-empty code lines are attributes that
/// include a `#[test]`-family attribute (`#[test]`, `#[tokio::test]`,
/// `#[test_case(...)]`, ...): the attribute path's final segment must start
/// with `test`.
fn preceded_by_test_attribute(code_lines: &[String]) -> bool {
    for line in code_lines.iter().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !line.starts_with("#[") {
            return false;
        }
        let path: &str = line[2..]
            .split([']', '(', '=', ' '])
            .next()
            .unwrap_or_default();
        if path
            .rsplit("::")
            .next()
            .is_some_and(|segment| segment.starts_with("test"))
        {
            return true;
        }
    }
    false
}
