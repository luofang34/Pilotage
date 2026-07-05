# ADR-0014: Protobuf as the wire-schema source of truth

- Status: Accepted
- Date: 2026-07-05

## Context

Every plane boundary carries versioned messages: control frames, telemetry,
authority events, capabilities, device profiles, and recorded events. Three forces
shape the format choice:

- **Independent deployment.** Peer-hosted session hosts and clients will not upgrade
  in lockstep; schema evolution must be safe across versions by construction.
- **Polyglot edges.** Adapters and gateways (ROS 2 bridges, MAVLink gateways,
  onboard companions) and debug tooling will not all be Rust, even though the core
  is.
- **Hot path.** Control frames flow at roughly 100–250 Hz per operator and telemetry
  somewhat faster; encoding must be compact and cheap, but this is not a
  zero-copy-or-die regime.

## Decision

- All cross-boundary messages are defined in **Protobuf** under `schemas/`, which is
  the single source of truth for the protocol.
- Rust code is generated at build time via `prost` into `pilotage-protocol`;
  generated code is neither hand-edited nor committed.
- Evolution rules: field numbers are never reused; unknown fields are tolerated;
  every enum has an `UNSPECIFIED = 0` sentinel; breaking changes require a new
  message or package version, negotiated at session setup.
- Framing: a WebTransport datagram carries exactly one envelope-wrapped protobuf
  message; streams carry length-delimited envelopes. The recording format adds
  explicit length-and-type framing.
- The canonical JSON mapping (protojson) MAY be used for diagnostics and
  configuration files, never on the real-time path.
- Once schemas exist, CI runs `buf lint` and `buf breaking` against the base branch
  (ADR-0015).

## Consequences

- Protocol changes are reviewed as API changes in one place (`schemas/`), with
  mechanical breaking-change detection.
- The hot path pays a small encode/decode cost; acceptable at these rates and
  message sizes (control frames are tens of bytes).
- Protobuf serialization is not canonical: replay, hashing, and signatures operate
  on recorded bytes, never on re-serialized messages.
- TypeScript types for browser debug tooling and any future non-Rust adapter come
  from the same schemas for free.

## Alternatives considered

- **serde + postcard/bincode:** lowest friction for Rust↔Rust and attractive with a
  wasm client, but it locks the protocol into Rust struct layout, and evolution
  discipline rests on convention — silent breakage across independently deployed
  versions is exactly the failure mode to exclude.
- **JSON everywhere:** transparent and debuggable, but allocation-heavy and verbose
  on the control/telemetry path; kept only as the diagnostic mapping.
- **FlatBuffers / Cap'n Proto:** zero-copy advantages don't pay off at these message
  sizes and rates, and Rust ergonomics plus review tooling are weaker than the
  protobuf ecosystem's.
