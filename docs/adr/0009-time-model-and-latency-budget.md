# ADR-0009: Explicit time model, end-to-end latency budget, stale-input rejection

- Status: Accepted
- Date: 2026-07-05

## Context

The user experience is the complete loop: simulation state → render → encode →
network → decode → presentation → human response → input sampling → network →
validation → simulation application. Average latency alone is insufficient — jitter,
frame age, queue growth, and tail latency determine controllability. Additionally,
accelerated training (ADR-0013) breaks any assumption that simulation time tracks
wall-clock time, so time domains must be explicit from the start.

## Decision

### Time model

The protocol distinguishes three time domains; they are never conflated:

- `transport_time` — monotonic clock local to each endpoint; used for sampling
  timestamps, age estimation, and staleness decisions. Never compared raw across
  endpoints; offset/RTT estimation handles cross-endpoint correlation.
- `host_time` — the session host's monotonic clock; the reference for per-session
  event ordering and latency accounting.
- `simulation_time` — the adapter's simulated clock (ticks); may run slower, faster,
  or stepped relative to wall clock.

### Control-frame requirements

Every real-time control frame MUST include: scope, fencing generation, sequence
number, client monotonic sample timestamp, profile revision, and the current logical
input state with required edge events.

The session host MUST record or derive: receive timestamp, estimated frame age,
simulation application tick, and the accepted / transformed / constrained / rejected
outcome — plus video capture and presentation timing signals where available.

### Rejection rules

The host MUST reject: stale generations; duplicates and reordered frames according to
each message class's semantics (ADR-0011); and frames older than the configured
maximum control age. Continuous axes use latest-valid-value semantics; one-shot and
button-edge commands use explicit consumption or acknowledgement semantics.

### Initial engineering budget (targets, not guarantees, until validated)

| Stage | Initial target |
|---|---:|
| Device sampling → serialized frame | < 4–8 ms under normal load |
| Control one-way network path | < 20–40 ms preferred |
| Host validation → simulator application | ≤ 1 simulation tick in the normal path |
| Render + encode | < 16–30 ms preferred |
| Video network + jitter buffer | < 30–60 ms preferred |
| Decode + presentation | < 16–30 ms preferred |
| Machine closed loop (excluding human) | < 100–150 ms design target |

## Consequences

- Latency observability is a core product requirement; every stage above is
  instrumented via structured events (ADR-0012).
- Local-loopback mode is the baseline that separates software latency from network
  latency; it is measured first in every increment.
- Every queue in the real-time path has an explicit maximum depth and drop policy;
  drops are counted, never silent.
- The validation suite MUST include jitter, loss, reordering, bandwidth reduction,
  encoder overload, browser main-thread contention, and simulator tick slowdown.
- p95/p99 closed-loop targets under specified network profiles remain to be set
  after first measurements (tracked in the backlog).
