# ADR-0001: Record architecture decisions as versioned files in this repository

- Status: Accepted
- Date: 2026-07-05

## Context

Pilotage spans protocol design, authority semantics, real-time transport, simulator
integration, and future decentralized deployment. Decisions made now will be
re-examined by contributors who need the reasoning, not just the outcome. A single
monolithic design document (the pre-repository draft) proved hard to evolve: unrelated
decisions shared one version number and one review cycle, and individual decisions
could not be superseded independently.

## Decision

- Every significant architectural commitment is recorded as one file under
  `docs/adr/`, named `NNNN-short-slug.md`, numbered in acceptance order.
- Records use a compact MADR-style template: Status, Date, Context, Decision,
  Consequences, and — where a real choice was weighed — Alternatives considered.
- Statuses: `Proposed`, `Accepted`, `Deprecated`, `Superseded by ADR-NNNN`.
- An Accepted record is immutable except for status changes and factual errata. A
  changed decision gets a new record that supersedes the old one; history is never
  rewritten.
- Requirement words `MUST`, `SHOULD`, `MAY` follow RFC 2119 usage.
- Open questions listed inside a record are commitments to decide, not decisions;
  resolving one produces either an amendment before acceptance or a new record.

## Consequences

- Design review happens per decision, matching the one-issue-per-PR discipline.
- The index in `docs/adr/README.md` is the entry point and MUST stay current.
- The pre-repository draft (v0.3) is superseded by these records; the provenance
  table in the index maps its sections to their successors.
