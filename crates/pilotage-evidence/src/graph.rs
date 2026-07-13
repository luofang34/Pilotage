//! The evidence graph: the maintained engineering record.
//!
//! Nodes are keyed by [`NodeId`] and edges are held in a sorted set, so two
//! graphs with the same content compare and serialize identically regardless of
//! authoring order. A single [`Graph::reachable_via`] primitive drives every
//! traversal the gate and the impact report need.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::id::NodeId;
use crate::node::{Node, NodeKind};
use crate::relation::{Edge, RelationKind};
use crate::scope::{Exception, Scope};

/// A rejected attempt to insert a second node under an existing identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DuplicateNode(pub NodeId);

/// The maintained engineering record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Graph {
    /// The schema version the graph was authored against.
    pub version: u32,
    /// The declared assurance scope.
    pub scope: Scope,
    nodes: BTreeMap<NodeId, Node>,
    edges: BTreeSet<Edge>,
    exceptions: Vec<Exception>,
}

impl Graph {
    /// An empty graph for the given version and scope.
    #[must_use]
    pub fn new(version: u32, scope: Scope) -> Self {
        Self {
            version,
            scope,
            nodes: BTreeMap::new(),
            edges: BTreeSet::new(),
            exceptions: Vec::new(),
        }
    }

    /// Inserts a node, rejecting a duplicate identifier.
    ///
    /// # Errors
    ///
    /// Returns [`DuplicateNode`] if a node with the same id already exists.
    pub(crate) fn insert_node(&mut self, node: Node) -> Result<(), DuplicateNode> {
        if self.nodes.contains_key(&node.id) {
            return Err(DuplicateNode(node.id));
        }
        self.nodes.insert(node.id.clone(), node);
        Ok(())
    }

    /// Adds an edge (a duplicate edge is silently collapsed).
    pub(crate) fn add_edge(&mut self, edge: Edge) {
        self.edges.insert(edge);
    }

    /// Adds an exception record.
    pub(crate) fn add_exception(&mut self, exception: Exception) {
        self.exceptions.push(exception);
    }

    /// Whether the graph holds no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The node with the given id, if present.
    #[must_use]
    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Whether a node with the given id exists.
    #[must_use]
    pub fn contains(&self, id: &NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    /// All nodes, ordered by id.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    /// All edges, in canonical (sorted) order.
    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        self.edges.iter()
    }

    /// All exception records.
    pub fn exceptions(&self) -> &[Exception] {
        &self.exceptions
    }

    /// The ids of nodes of a given kind, ordered.
    pub fn ids_of_kind(&self, kind: NodeKind) -> impl Iterator<Item = &NodeId> {
        self.nodes
            .values()
            .filter(move |n| n.kind == kind)
            .map(|n| &n.id)
    }

    /// The kind of the node with this id, if present.
    #[must_use]
    pub fn kind_of(&self, id: &NodeId) -> Option<NodeKind> {
        self.nodes.get(id).map(|n| n.kind)
    }

    /// The set of nodes reachable from `start` by walking `forward` relations
    /// in their declared direction (`from` -> `to`) and `reverse` relations
    /// against it (`to` -> `from`). `start` itself is not included.
    #[must_use]
    pub fn reachable_via(
        &self,
        start: &NodeId,
        forward: &[RelationKind],
        reverse: &[RelationKind],
    ) -> BTreeSet<NodeId> {
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(start.clone());
        while let Some(current) = queue.pop_front() {
            for edge in &self.edges {
                let next = if edge.from == current && forward.contains(&edge.relation) {
                    Some(&edge.to)
                } else if edge.to == current && reverse.contains(&edge.relation) {
                    Some(&edge.from)
                } else {
                    None
                };
                if let Some(next) = next
                    && next != start
                    && seen.insert(next.clone())
                {
                    queue.push_back(next.clone());
                }
            }
        }
        seen
    }
}
