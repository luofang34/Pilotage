# ADR-0031: Navigation guidance rides telemetry as its own stamped role; deviation scaling is display policy

- Status: Proposed
- Date: 2026-07-29

## Context

The instrument runtime already models lateral navigation display — a
selected `NavSource`, TO/FROM resolution, course with its declared north,
deviation in dots, and distance — and the HSI draws it when fed. Nothing
feeds it: the mission executor ([ADR-0025](0025-client-optional-operation-automation-principals.md))
holds the active leg, desired course, cross-track and vertical deviation,
and distance-to-waypoint, but that state never leaves the host. The
telemetry plane's pattern for exactly this situation is established:
independent sub-messages under their own `MeasurementStamp` (source
identity, epoch, wrapping sequence, acquisition time, clock, role,
integrity), gated fail-closed at the client ingress
([ADR-0018](0018-avionics-telemetry-and-aviate-adapter.md)).
[ADR-0024](0024-navigation-authority-boundary.md) already commits the
navigation solution to joining the source-role vocabulary as its own
role, never relabeled as FC state or simulation truth.

## Decision

- **A `NavGuidanceState` sub-message joins `TelemetrySample`**,
  additively, under its own `MeasurementStamp` with a new
  `SOURCE_ROLE_NAVIGATION_SOLUTION`. It carries the active-leg geometry
  in raw canonical units: TO/FROM identifiers, desired course in radians
  with true-north reference, lateral deviation in meters (positive right
  of course, the geodesy convention), vertical deviation in meters
  (positive above profile; absent without a vertical constraint),
  distance to the active waypoint in meters, leg index and waypoint
  count, and the backing solution quality.
- **The producer is the navigation/mission component, never the
  adapter.** The mission executor publishes guidance state to the host's
  telemetry assembly through a host-internal side input; the FC adapter
  contract is untouched. On a vehicle without an active plan the field is
  absent — absence means no guidance, and receivers remove the deviation
  display rather than centering it.
- **Deviation scaling to dots is display policy**
  ([ADR-0017](0017-instrument-display-runtime.md)): the wire carries
  meters; the client's display profile converts to the instrument
  model's dot scale (full-scale deflection per vehicle class). No dots
  cross the wire.
- **Ingress gates the new role like every other group**: wrap-aware
  sequence and epoch admission per source, role checked explicitly
  (guidance is display context — never an input to control validation or
  a fallback for a missing estimate), freshness aged from the stamp, and
  `solution_quality` unusable removes the display.
- **This is the extensible-group pattern** ([ADR-0029](0029-panel-layout-look-plugins.md)):
  one typed, stamped, role-gated signal group added end to end without
  touching any other group — the shape every future instrument-cluster
  signal (engine data, traffic, terrain bands) follows.

## Consequences

- The HSI course pointer, CDI, TO ident, and distance readout come alive
  in any session with an active mission — simulation today, and the same
  path serves a real vehicle whose Navigate component publishes guidance.
- `buf breaking` passes by construction; clients that predate the field
  ignore it.
- The dots-per-meter display profile is a client-side named constant
  with tests, adjustable per airframe class without wire change.
- A future moving map consumes the same group plus navdata snapshots
  ([ADR-0030](0030-communicate-navdata-provisioning.md)); no second
  guidance vocabulary is introduced for it.

## Alternatives considered

- **Publishing dots on the wire:** rejected; display scaling is per
  airframe and per panel, and baking it into telemetry couples every
  display to one deflection policy.
- **Deriving deviation client-side from position plus plan:** rejected
  for now; it requires shipping the active plan and duplicating leg
  geometry in every client, and the executor's own numbers are the
  truth being flown. Revisit when clients plan independently (EFB).
