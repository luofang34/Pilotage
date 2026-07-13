//! Unit tests for the text grammar and its fail-closed syntax errors.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use crate::node::NodeKind;
use crate::parse::{ParseError, parse_graph};
use crate::testkit::VALID_SLICE;

#[test]
fn the_slice_parses_with_every_node() {
    let graph = parse_graph(VALID_SLICE).expect("parses");
    assert_eq!(graph.version, 1);
    assert_eq!(graph.scope.id, "ATT-01");
    assert_eq!(graph.nodes().count(), 11);
    assert_eq!(graph.scope.requirements.len(), 2);
    let hazard = graph.kind_of(&crate::id::NodeId::new("FC-ATT-06").unwrap());
    assert_eq!(hazard, Some(NodeKind::Hazard));
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    let text = "# a comment\n\nevidence-graph 1\n# another\nscope S\n\nnode A design\n";
    let graph = parse_graph(text).expect("parses");
    assert_eq!(graph.nodes().count(), 1);
}

#[test]
fn missing_header_is_rejected() {
    let err = parse_graph("scope S\nnode A design\n").unwrap_err();
    assert_eq!(err, ParseError::MissingHeader);
}

#[test]
fn missing_scope_is_rejected() {
    let err = parse_graph("evidence-graph 1\nnode A design\n").unwrap_err();
    assert_eq!(err, ParseError::MissingScope);
}

#[test]
fn unsupported_version_is_rejected() {
    let err = parse_graph("evidence-graph 2\nscope S\n").unwrap_err();
    assert!(matches!(err, ParseError::UnsupportedVersion { .. }));
}

#[test]
fn duplicate_node_is_rejected() {
    let text = "evidence-graph 1\nscope S\nnode A design\nnode A hazard\n";
    let err = parse_graph(text).unwrap_err();
    assert!(matches!(err, ParseError::DuplicateNode { .. }));
}

#[test]
fn unknown_kind_is_rejected() {
    let err = parse_graph("evidence-graph 1\nscope S\nnode A widget\n").unwrap_err();
    assert!(matches!(err, ParseError::UnknownKind { .. }));
}

#[test]
fn unknown_relation_is_rejected() {
    let text = "evidence-graph 1\nscope S\nnode A design\nnode B design\nedge A owns B\n";
    let err = parse_graph(text).unwrap_err();
    assert!(matches!(err, ParseError::UnknownRelation { .. }));
}

#[test]
fn a_field_before_any_record_is_rejected() {
    let err = parse_graph("evidence-graph 1\nscope S\ntitle stray\n").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedField { .. }));
}

#[test]
fn a_node_field_under_an_exception_is_rejected() {
    let text = "evidence-graph 1\nscope S\nexception E\ncovers A\ntitle no\n";
    let err = parse_graph(text).unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedField { .. }));
}

#[test]
fn an_exception_without_covers_is_rejected() {
    let text = "evidence-graph 1\nscope S\nexception E\nowner me\n";
    let err = parse_graph(text).unwrap_err();
    assert!(matches!(err, ParseError::ExceptionMissingCovers { .. }));
}

#[test]
fn a_bad_identifier_is_rejected() {
    let err = parse_graph("evidence-graph 1\nscope S\nnode bad@id design\n").unwrap_err();
    assert!(matches!(err, ParseError::InvalidId { .. }));
}

#[test]
fn trailing_tokens_on_an_edge_are_rejected() {
    let text = "evidence-graph 1\nscope S\nnode A design\nnode B design\nedge A covers B C\n";
    let err = parse_graph(text).unwrap_err();
    assert!(matches!(err, ParseError::TrailingTokens { .. }));
}
