# ADR-0012: Structured session events for observability, recording, and replay

- Status: Accepted
- Date: 2026-07-05

## Context

Latency and authority failures cannot be diagnosed from ordinary application logs.
The system must distinguish operator intent, accepted authority, network arrival,
host validation, simulator application, telemetry, and rendered-video timing — and
correlate them across plane boundaries.

## Decision

- A versioned structured event model covers, at minimum: session and host lifecycle
  (`SessionCreated`, `HostRegistered`, `UserJoined`, `CapabilityIssued`), authority
  (`ScopeLeaseGranted`, `ScopeTransferOffered`, `ScopeTransferCommitted`,
  `ScopeLeaseRevoked`, `EmergencyOverrideApplied`), the control path
  (`ControlReceived`, `ControlRejected`, `ControlApplied`), and the sensing path
  (`TelemetryObserved`, `VideoFrameCaptured`, `MediaTimingObserved`,
  `LinkStateChanged`, `VehicleFailoverApplied`, `WarningRaised`).
- Events distinguish the stages of one intent: client-observed device input →
  normalized operator intent → host-received control → authority-valid control →
  simulator-applied control → resulting telemetry. Correlation identifiers tie the
  chain together (ADR-0003, ADR-0009).
- Event production MUST be non-blocking with respect to the real-time path: bounded
  queues, and when a consumer lags, drops are **counted, logged at error level, and
  surfaced as a health signal** — a lagging observer is a correctness signal, never
  silent noise.
- Persistence MAY be sampled or disabled by policy for high-rate control and
  telemetry events. Authority events are always persisted: they are the audit trail
  for handover and override disputes.
- Replay tooling targets the deterministic reference adapter (ADR-0008) first;
  Gazebo scenario replay is supported where determinism permits.

## Consequences

- The event schema and correlation identifiers are part of the base protocol design
  (ADR-0014), not an add-on.
- High-rate events may need chunked binary storage or aggregation; the recording
  format supports both event streams and dense batch trajectories (ADR-0013).
- Privacy and retention policy must be resolved before production recording
  (backlog).
- Because core logic is sans-IO (ADR-0002), replaying a recorded session through the
  state machines reproduces authority transitions and applied-control ordering
  exactly; conformance tests assert this.
