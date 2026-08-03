# ADR-0024: Control-grade estimation stays with the flight controller; Navigate owns the global solution and guidance

- Status: Proposed
- Date: 2026-07-29

## Context

The FC owns an estimator sized for stabilization: high-rate, sensor-direct,
with externally supplied inputs bounded and removable by its own specification
(GNSS-class inputs never drive inner attitude/rate loops; on unrecoverable
fault it degrades monotonically and waits for external supervision). Navigate
([ADR-0023](0023-vehicle-side-decomposition-fc-navigate-communicate.md)) fuses
heterogeneous sources — GNSS, celestial, visual, and others — into a
navigation-grade global solution and executes flight plans against it. The two
estimates serve different loops and different failure budgets; blurring them
would let a navigation fault propagate into stabilization.

## Decision

- **The FC owns the control-grade estimator** required for stabilization and
  remains fully operational without Navigate.
- **Navigate owns the global navigation charter enumerated in
  [ADR-0023](0023-vehicle-side-decomposition-fc-navigate-communicate.md)** —
  multi-sensor fusion, integrity assessment, flight-plan management and
  execution, guidance, and terrain awareness; that record's list is
  authoritative, this record decides the boundary rules.
- **Guidance interface:** Navigate flies the vehicle by streaming setpoints
  through the FC's declared command surface (position and velocity setpoints,
  deviation tracking, as the FC declares them); a trajectory-level handoff,
  if ever adopted, is an FC-side surface extension, not an assumption. These
  commands enter through the host's fenced control path as an
  automation-class principal
  ([ADR-0025](0025-client-optional-operation-automation-principals.md)); the
  FC does not distinguish Navigate from any other commander.
- **Aiding interface:** Navigate MAY supply timestamped, bounded aiding
  observations with covariance and integrity metadata. The FC independently
  validates, fuses, or rejects each observation; acceptance is never assumed,
  and a rejected observation changes nothing about guidance authority.
- **Correlation rule:** fused outputs derived from shared measurements MUST
  NOT be treated as independent aids. Every aiding observation declares its
  source composition; an observation that ingests measurements the FC also
  consumes directly (the same GNSS receiver, FC-exported state) is either
  excluded or declared correlated so the FC can discount it.
- **Placement of learned components:** visual navigation belongs to Navigate.
  If latency-critical learned control policies are adopted, they execute
  within the FC's control architecture under its supervision discipline
  (degradation ladder, envelope protection) — an FC-side extension to be
  specified with the FC — never in Navigate.
- **Integrity:** the navigation solution carries integrity and quality
  metadata with every estimate. Guidance decisions that require integrity
  fail closed when it is absent or degraded; terrain awareness continues on
  best-available data with its quality shown explicitly.
- **Displays:** the navigation solution joins the telemetry source-role
  vocabulary as its own role — never relabeled as FC state or simulation
  truth.

## Consequences

- Two estimators exist by design. Divergence between them is a monitored,
  displayable condition — not an error to be hidden by picking one silently.
- The aiding-observation schema (reference frames, covariance encoding,
  integrity terms, source composition) is an interface RFC owed jointly with
  the Aviate side, like the native-link RFC deferred in
  [ADR-0019](0019-pluggable-vehicle-link-shm-first.md).
- Single-source operation degrades gracefully: with only one navigation
  source, Navigate's integrity output narrows honestly rather than
  fabricating confidence; with no source, guidance refuses and the FC keeps
  flying on its own estimator.
- Terrain awareness (EGPWS-class) binds Navigate's solution to signed terrain
  packages (`pilotage-svs-db`) independently of the FC estimate.

## Alternatives considered

- **Navigate as the FC's navigation authority** (FC outer loops consume the
  fused solution): rejected — a navigation fault would propagate directly
  into the control loop, and it contradicts the FC's
  bounded-external-influence specification.
- **Guidance-only with no aiding path:** rejected as the only mode; it
  remains the degenerate case whenever no aiding is offered or every
  observation is rejected.
