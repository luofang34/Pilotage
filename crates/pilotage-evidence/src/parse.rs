//! The text grammar the evidence graph is authored in and parsed from.
//!
//! The format is line-oriented so it diffs cleanly and parses without any
//! external serializer. Blank lines and `#` comments are ignored. Each line is
//! either a *record* / *directive* introduced by a keyword, or a *field* that
//! attaches to the record currently being built:
//!
//! ```text
//! evidence-graph 1
//! scope ATT-01
//! scope-title extreme-attitude behavior traceability slice
//! scope-requirement AIR-ENV-002
//!
//! node AIR-ENV-002 safety-requirement
//! title Attitude envelope determinism
//! locator docs/instruments/requirements.md#air-env-002
//! attr safety-impact catastrophic-if-credited
//!
//! edge AIR-ENV-002 mitigates FC-ATT-06
//!
//! exception EX-1
//! covers AIR-MODE-003
//! owner sokoly
//! rationale downstream result pending
//! status open
//! expiry 2026-09-01
//! review REVIEW-AIR01
//! ```
//!
//! Parsing checks syntax only: well-formed ids, known kinds and relations, no
//! duplicate node id. Semantic checks — dangling edges, orphans, unresolved
//! selectors — belong to the gate, which fails closed on them.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::graph::Graph;
use crate::id::{IdError, NodeId};
use crate::node::{Node, NodeKind};
use crate::relation::{Edge, RelationKind};
use crate::scope::{Exception, Scope};

/// The only schema version this parser accepts.
const SUPPORTED_VERSION: u32 = 1;

/// A syntactic failure parsing evidence-graph text.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    /// The `evidence-graph <version>` header line was absent.
    #[error("missing 'evidence-graph <version>' header")]
    MissingHeader,
    /// No `scope <id>` directive was present.
    #[error("missing 'scope <id>' directive")]
    MissingScope,
    /// The schema version is not supported.
    #[error("unsupported schema version {found} (line {line}); expected {SUPPORTED_VERSION}")]
    UnsupportedVersion {
        /// The version read.
        found: String,
        /// The 1-based line number.
        line: usize,
    },
    /// A line began with an unknown keyword.
    #[error("unknown keyword {keyword:?} on line {line}")]
    UnknownKeyword {
        /// The keyword.
        keyword: String,
        /// The 1-based line number.
        line: usize,
    },
    /// A keyword was missing a required argument.
    #[error("keyword {keyword:?} on line {line} is missing an argument")]
    MissingArgument {
        /// The keyword.
        keyword: String,
        /// The 1-based line number.
        line: usize,
    },
    /// A line carried more tokens than its keyword accepts.
    #[error("keyword {keyword:?} on line {line} has trailing tokens {extra:?}")]
    TrailingTokens {
        /// The keyword.
        keyword: String,
        /// The unexpected remainder.
        extra: String,
        /// The 1-based line number.
        line: usize,
    },
    /// An unknown node kind token.
    #[error("unknown node kind {token:?} on line {line}")]
    UnknownKind {
        /// The token.
        token: String,
        /// The 1-based line number.
        line: usize,
    },
    /// An unknown relation token.
    #[error("unknown relation {token:?} on line {line}")]
    UnknownRelation {
        /// The token.
        token: String,
        /// The 1-based line number.
        line: usize,
    },
    /// A malformed identifier.
    #[error("invalid identifier on line {line}")]
    InvalidId {
        /// The 1-based line number.
        line: usize,
        /// The identifier failure.
        #[source]
        source: IdError,
    },
    /// A field line appeared where no record (or the wrong record) was open.
    #[error("field {field:?} on line {line} is not valid in the current context")]
    UnexpectedField {
        /// The field keyword.
        field: String,
        /// The 1-based line number.
        line: usize,
    },
    /// The same attribute key was set twice on one node.
    #[error("duplicate attribute {key:?} on line {line}")]
    DuplicateAttr {
        /// The attribute key.
        key: String,
        /// The 1-based line number.
        line: usize,
    },
    /// Two nodes shared an identifier.
    #[error("duplicate node id {id:?} on line {line}")]
    DuplicateNode {
        /// The repeated id.
        id: String,
        /// The 1-based line number of the second definition.
        line: usize,
    },
    /// An exception record never declared what it covers.
    #[error("exception {id:?} does not declare 'covers'")]
    ExceptionMissingCovers {
        /// The exception id.
        id: String,
    },
}

/// Parses evidence-graph text into a [`Graph`].
///
/// # Errors
///
/// Returns [`ParseError`] on any syntactic fault; the graph is never partially
/// built past a fault.
pub fn parse_graph(text: &str) -> Result<Graph, ParseError> {
    let mut builder = Builder::default();
    for (index, raw) in text.lines().enumerate() {
        builder.line(index + 1, raw.trim())?;
    }
    builder.finish()
}

/// A partially built node or exception awaiting its field lines.
#[derive(Default)]
enum Pending {
    #[default]
    None,
    Node {
        line: usize,
        node: Node,
    },
    Exception {
        builder: ExceptionBuilder,
    },
}

/// Accumulates records until [`Builder::finish`] assembles the graph.
#[derive(Default)]
struct Builder {
    version: Option<u32>,
    scope_id: Option<String>,
    scope_title: String,
    scope_reqs: BTreeSet<NodeId>,
    nodes: Vec<(usize, Node)>,
    edges: Vec<Edge>,
    exceptions: Vec<Exception>,
    pending: Pending,
}

impl Builder {
    /// Processes one already-trimmed line.
    fn line(&mut self, line: usize, text: &str) -> Result<(), ParseError> {
        if text.is_empty() || text.starts_with('#') {
            return Ok(());
        }
        let (keyword, rest) = split_first(text);
        if is_field_keyword(keyword) {
            return self.field(line, keyword, rest);
        }
        self.flush()?;
        self.record(line, keyword, rest)
    }

    /// Handles a record or directive line, having flushed any open record.
    fn record(&mut self, line: usize, keyword: &str, rest: &str) -> Result<(), ParseError> {
        match keyword {
            "evidence-graph" => self.version = Some(parse_version(rest, line)?),
            "scope" => self.scope_id = Some(one_token(keyword, rest, line)?.to_string()),
            "scope-title" => self.scope_title = rest.to_string(),
            "scope-requirement" => {
                let id = node_id(one_token(keyword, rest, line)?, line)?;
                self.scope_reqs.insert(id);
            }
            "node" => {
                self.pending = Pending::Node {
                    line,
                    node: parse_node(rest, line)?,
                }
            }
            "edge" => self.edges.push(parse_edge(rest, line)?),
            "exception" => {
                let id = one_token(keyword, rest, line)?.to_string();
                self.pending = Pending::Exception {
                    builder: ExceptionBuilder::new(id),
                };
            }
            other => {
                return Err(ParseError::UnknownKeyword {
                    keyword: other.to_string(),
                    line,
                });
            }
        }
        Ok(())
    }

    /// Attaches a field line to the open record.
    fn field(&mut self, line: usize, keyword: &str, rest: &str) -> Result<(), ParseError> {
        match &mut self.pending {
            Pending::Node { node, .. } => apply_node_field(node, keyword, rest, line),
            Pending::Exception { builder } => builder.field(keyword, rest, line),
            Pending::None => Err(ParseError::UnexpectedField {
                field: keyword.to_string(),
                line,
            }),
        }
    }

    /// Commits the open record, if any.
    fn flush(&mut self) -> Result<(), ParseError> {
        match std::mem::take(&mut self.pending) {
            Pending::None => {}
            Pending::Node { line, node } => self.nodes.push((line, node)),
            Pending::Exception { builder } => self.exceptions.push(builder.build()?),
        }
        Ok(())
    }

    /// Assembles the graph, rejecting a duplicate node id.
    fn finish(mut self) -> Result<Graph, ParseError> {
        self.flush()?;
        let version = self.version.ok_or(ParseError::MissingHeader)?;
        let scope_id = self.scope_id.ok_or(ParseError::MissingScope)?;
        let mut scope = Scope::new(scope_id, self.scope_title);
        scope.requirements = self.scope_reqs;
        let mut graph = Graph::new(version, scope);
        for (line, node) in self.nodes {
            graph
                .insert_node(node)
                .map_err(|dup| ParseError::DuplicateNode {
                    id: dup.0.to_string(),
                    line,
                })?;
        }
        for edge in self.edges {
            graph.add_edge(edge);
        }
        for exception in self.exceptions {
            graph.add_exception(exception);
        }
        Ok(graph)
    }
}

/// Collects an exception's fields before its `covers` target is confirmed.
struct ExceptionBuilder {
    id: String,
    covers: Option<NodeId>,
    owner: String,
    rationale: String,
    status: String,
    expiry: String,
    review: Option<NodeId>,
}

impl ExceptionBuilder {
    fn new(id: String) -> Self {
        Self {
            id,
            covers: None,
            owner: String::new(),
            rationale: String::new(),
            status: String::new(),
            expiry: String::new(),
            review: None,
        }
    }

    fn field(&mut self, keyword: &str, rest: &str, line: usize) -> Result<(), ParseError> {
        match keyword {
            "covers" => self.covers = Some(node_id(one_token(keyword, rest, line)?, line)?),
            "owner" => self.owner = rest.to_string(),
            "rationale" => self.rationale = rest.to_string(),
            "status" => self.status = rest.to_string(),
            "expiry" => self.expiry = rest.to_string(),
            "review" => self.review = Some(node_id(one_token(keyword, rest, line)?, line)?),
            other => {
                return Err(ParseError::UnexpectedField {
                    field: other.to_string(),
                    line,
                });
            }
        }
        Ok(())
    }

    fn build(self) -> Result<Exception, ParseError> {
        let covers = self.covers.ok_or(ParseError::ExceptionMissingCovers {
            id: self.id.clone(),
        })?;
        Ok(Exception {
            id: self.id,
            covers,
            owner: self.owner,
            rationale: self.rationale,
            status: self.status,
            expiry: self.expiry,
            review: self.review,
        })
    }
}

fn is_field_keyword(keyword: &str) -> bool {
    matches!(
        keyword,
        "title"
            | "locator"
            | "attr"
            | "covers"
            | "owner"
            | "rationale"
            | "status"
            | "expiry"
            | "review"
    )
}

fn apply_node_field(
    node: &mut Node,
    keyword: &str,
    rest: &str,
    line: usize,
) -> Result<(), ParseError> {
    match keyword {
        "title" => node.title = rest.to_string(),
        "locator" => node.locator = Some(rest.to_string()),
        "attr" => {
            let (key, value) = split_first(rest);
            if key.is_empty() {
                return Err(ParseError::MissingArgument {
                    keyword: "attr".to_string(),
                    line,
                });
            }
            if node
                .attrs
                .insert(key.to_string(), value.to_string())
                .is_some()
            {
                return Err(ParseError::DuplicateAttr {
                    key: key.to_string(),
                    line,
                });
            }
        }
        other => {
            return Err(ParseError::UnexpectedField {
                field: other.to_string(),
                line,
            });
        }
    }
    Ok(())
}

fn parse_node(rest: &str, line: usize) -> Result<Node, ParseError> {
    let (id_tok, tail) = split_first(rest);
    let (kind_tok, extra) = split_first(tail);
    if id_tok.is_empty() || kind_tok.is_empty() {
        return Err(ParseError::MissingArgument {
            keyword: "node".to_string(),
            line,
        });
    }
    reject_trailing("node", extra, line)?;
    let id = node_id(id_tok, line)?;
    let kind = NodeKind::from_token(kind_tok).ok_or_else(|| ParseError::UnknownKind {
        token: kind_tok.to_string(),
        line,
    })?;
    Ok(Node::new(id, kind, String::new()))
}

fn parse_edge(rest: &str, line: usize) -> Result<Edge, ParseError> {
    let (from_tok, tail) = split_first(rest);
    let (rel_tok, tail) = split_first(tail);
    let (to_tok, extra) = split_first(tail);
    if from_tok.is_empty() || rel_tok.is_empty() || to_tok.is_empty() {
        return Err(ParseError::MissingArgument {
            keyword: "edge".to_string(),
            line,
        });
    }
    reject_trailing("edge", extra, line)?;
    let relation =
        RelationKind::from_token(rel_tok).ok_or_else(|| ParseError::UnknownRelation {
            token: rel_tok.to_string(),
            line,
        })?;
    Ok(Edge::new(
        node_id(from_tok, line)?,
        relation,
        node_id(to_tok, line)?,
    ))
}

fn parse_version(rest: &str, line: usize) -> Result<u32, ParseError> {
    let token = one_token("evidence-graph", rest, line)?;
    let version = token
        .parse::<u32>()
        .map_err(|_| ParseError::UnsupportedVersion {
            found: token.to_string(),
            line,
        })?;
    if version != SUPPORTED_VERSION {
        return Err(ParseError::UnsupportedVersion {
            found: token.to_string(),
            line,
        });
    }
    Ok(version)
}

fn node_id(token: &str, line: usize) -> Result<NodeId, ParseError> {
    NodeId::new(token).map_err(|source| ParseError::InvalidId { line, source })
}

/// Requires the remainder to be exactly one token.
fn one_token<'a>(keyword: &str, rest: &'a str, line: usize) -> Result<&'a str, ParseError> {
    let (token, extra) = split_first(rest);
    if token.is_empty() {
        return Err(ParseError::MissingArgument {
            keyword: keyword.to_string(),
            line,
        });
    }
    reject_trailing(keyword, extra, line)?;
    Ok(token)
}

fn reject_trailing(keyword: &str, extra: &str, line: usize) -> Result<(), ParseError> {
    if extra.is_empty() {
        Ok(())
    } else {
        Err(ParseError::TrailingTokens {
            keyword: keyword.to_string(),
            extra: extra.to_string(),
            line,
        })
    }
}

/// Splits off the first whitespace-delimited token, returning it and the
/// left-trimmed remainder.
fn split_first(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    match text.find(char::is_whitespace) {
        Some(idx) => (&text[..idx], text[idx..].trim_start()),
        None => (text, ""),
    }
}

#[cfg(test)]
mod tests;
