# ADR-0011: Separate control, telemetry, authority-event, and bulk message classes

- Status: Accepted
- Date: 2026-07-05

## Context

Continuous axis samples, camera commands, telemetry snapshots, authority changes,
emergency actions, and configuration objects have different reliability, ordering,
and freshness needs. One reliable ordered stream would create head-of-line blocking
and unpredictable overload behavior exactly when the link degrades.

## Decision

- Messages are grouped into classes with explicit delivery semantics, mapped onto the
  channel layout of ADR-0005:
  - **Continuous control** — superseded samples MAY be dropped; only freshness
    matters.
  - **Edges and one-shots** — button edges and one-shot commands are preserved under
    explicit consumption/acknowledgement semantics; never silently dropped.
  - **Authority and mode events** — reliable, ordered, idempotent.
  - **Telemetry** — best-effort by default; individual fields may declare
    reliability requirements.
  - **Bulk configuration** — reliable, ordered, latency-insensitive.
- Telemetry stays separate from encoded video; telemetry and authority overlays are
  rendered client-side by default, so stale data is never baked into encoded frames.
- Camera-control commands are ordinary scoped control messages under the camera
  scope — not media-plane implementation details — so the camera helm can be held
  independently of the vehicle helm.

## Consequences

- Each message class requires an explicit queue, overload, and drop policy; drops
  are counted per class (ADR-0009, ADR-0012).
- Telemetry schemas MUST state units, reference frames, validity, and update-rate
  expectations (published via adapter capabilities, ADR-0008).
- Class semantics are part of the protocol contract (ADR-0014), independent of the
  transport that carries them.
