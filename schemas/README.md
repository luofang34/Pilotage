# schemas/ — wire-schema source of truth

Protobuf definitions for every cross-boundary message
([ADR-0014](../docs/adr/0014-protobuf-wire-schema.md)): protocol envelopes, control
frames, telemetry, authority events, host capabilities, device profiles, and the
recorded-event format.

Rules:

- This directory is the single source of truth; Rust code is generated at build time
  (`prost`) and never committed or hand-edited.
- Field numbers are never reused; unknown fields are tolerated; enums carry an
  `UNSPECIFIED = 0` sentinel.
- Schema changes are API changes: reviewed as such, and gated by `buf lint` and
  `buf breaking` in CI once populated.
