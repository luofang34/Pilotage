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
/// TOP-LEVEL `function <symbol>(` definition AND membership in a top-level
/// `for (const <v> of [ ... ])` runner array — the registration construct
/// these suites use to actually invoke their tests. Requiring both, at
/// top level, mirrors the Rust rule's "definition plus `#[test]`": a
/// nested definition, a helper defined but never registered, a name only
/// referenced by a call (self-recursion or another helper, which are
/// invocations, not runner-array elements), and every comment or
/// string-literal decoy (single-, double-, or backtick-quoted) all fail
/// to resolve. All string and comment content is blanked before matching.
pub(super) fn defines_js(text: &str, symbol: &str) -> bool {
    let code = strip_js_literals(text);
    top_level_function(&code, symbol) && registered_in_runner(&code, symbol)
}

/// Blanks every comment and string literal (single-, double-, and
/// backtick-quoted, the latter including any `${…}` interpolation) to
/// spaces, preserving newlines and every structural character, so
/// definition and registration matching sees only real code and no decoy
/// hidden in a string or comment can resolve.
fn strip_js_literals(text: &str) -> String {
    #[derive(PartialEq)]
    enum State {
        Code,
        Line,
        Block,
        Str(char),
    }
    let mut state = State::Code;
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match &state {
            State::Code => match c {
                '/' if chars.peek() == Some(&'/') => state = State::Line,
                '/' if chars.peek() == Some(&'*') => state = State::Block,
                '"' | '\'' | '`' => {
                    state = State::Str(c);
                    out.push(' ');
                    continue;
                }
                _ => {
                    out.push(c);
                    continue;
                }
            },
            State::Line => {
                if c == '\n' {
                    state = State::Code;
                    out.push('\n');
                    continue;
                }
            }
            State::Block => {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    state = State::Code;
                    out.push_str("  ");
                    continue;
                }
            }
            State::Str(quote) => {
                if c == '\\' {
                    out.push(' ');
                    if let Some(next) = chars.next() {
                        out.push(if next == '\n' { '\n' } else { ' ' });
                    }
                    continue;
                }
                if c == *quote {
                    state = State::Code;
                    out.push(' ');
                    continue;
                }
            }
        }
        out.push(if c == '\n' { '\n' } else { ' ' });
    }
    out
}

/// Whether `code` contains a `function <symbol>(` definition at brace
/// depth zero — a top-level test, never a function nested inside another.
fn top_level_function(code: &str, symbol: &str) -> bool {
    let needle = format!("function {symbol}(");
    let mut depth: i32 = 0;
    let mut prev: Option<char> = None;
    for (i, c) in code.char_indices() {
        if depth == 0 && code[i..].starts_with(&needle) && prev.is_none_or(|p| !is_ident_char(p)) {
            return true;
        }
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        prev = Some(c);
    }
    false
}

/// Whether `symbol` is a whole-identifier element of some top-level
/// `for (… of [ … ])` array — the test runner. A `for` nested inside a
/// function body (a helper's own loop) is not top level and does not
/// count, so a test referenced only from a helper never resolves here.
fn registered_in_runner(code: &str, symbol: &str) -> bool {
    let mut depth: i32 = 0;
    let mut prev: Option<char> = None;
    for (i, c) in code.char_indices() {
        if depth == 0
            && code[i..].starts_with("for")
            && prev.is_none_or(|p| !is_ident_char(p))
            && let Some(array) = for_of_array(code, i + "for".len())
            && mentions_symbol(array, symbol)
        {
            return true;
        }
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        prev = Some(c);
    }
    false
}

/// The array-literal contents of a `for (… of [ … ])` header whose `for`
/// ends at `for_end`, or `None` for a C-style `for` or any header without
/// an `of [`. Only the header (up to its closing `)`) is scanned.
fn for_of_array(code: &str, for_end: usize) -> Option<&str> {
    let bytes = code.as_bytes();
    let mut i = skip_ws(bytes, for_end);
    if bytes.get(i) != Some(&b'(') {
        return None;
    }
    i += 1;
    let mut paren: i32 = 1;
    while i < bytes.len() && paren > 0 {
        if bytes[i] == b'o'
            && bytes.get(i + 1) == Some(&b'f')
            && (i == 0 || !is_ident_byte(bytes[i - 1]))
            && bytes.get(i + 2).is_none_or(|b| !is_ident_byte(*b))
        {
            let j = skip_ws(bytes, i + 2);
            if bytes.get(j) == Some(&b'[') {
                return capture_bracket(code, j);
            }
        }
        match bytes[i] {
            b'(' => paren += 1,
            b')' => paren -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

/// The contents between the `[` at `open` and its matching `]`.
fn capture_bracket(code: &str, open: usize) -> Option<&str> {
    let bytes = code.as_bytes();
    let mut depth: i32 = 0;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&code[open + 1..i]);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Advances past ASCII whitespace from `from`.
fn skip_ws(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
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
            .is_none_or(|c| !is_ident_char(c));
        let after = &code[pos + symbol.len()..];
        let after_ok = after.chars().next().is_none_or(|c| !is_ident_char(c));
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
