# ADR-0051: A physical vehicle flies a mission on its own estimate

- Status: Proposed
- Date: 2026-10-22

## Context

The mission executor ([ADR-0025](0025-client-optional-operation-automation-principals.md),
[ADR-0041](0041-one-mission-sequencing-core.md)) admits one ownship source:
a simulator's truth oracle. The engine synthesizes a GNSS-class fix from
each truth sample and fuses it. The mission document declares the
simulator target. A physical vehicle exports no truth oracle, so the
executor waits for a first solution for the whole mission.

A physical vehicle exports its own estimate: the position, height, and
ground velocity of its flight controller, under the operational-estimate
role of the telemetry vocabulary (LINK-04). This estimate is
estimator-derived. [ADR-0024](0024-navigation-authority-boundary.md)
forbids a fused output as an independent aid, and Navigate's fusion filter
refuses an observation that declares flight-controller state. The executor
must not present the estimate as a receiver fix.

A consumer multicopter measures its height from the point where its motors
last started, and its heading has no verified north reference. The
executor needs a declared way to place that height and to use that heading.

## Decision

### One admitted source, named in the configuration

The mission configuration names the ownship source. `simulation-truth`
keeps the fused truth path and is the default. `vehicle-estimate` admits
only samples under the operational-estimate role. Every other role is a
counted refusal in both modes. The host refuses a real-vehicle target with
the simulation-truth source at startup.

### The estimate is a pass-through solution, declared as such

Under `vehicle-estimate` the engine runs no fusion. The latest fresh sample
becomes the navigation solution. The solution declares its composition as
flight-controller state, carries the configured position and velocity
uncertainty, and grades degraded: one estimator, no redundancy, no fault
detection. A sample older than the configured age yields no solution, so
guidance refuses and the engine commands nothing. Display guidance shows
the degraded quality.

### The execution target follows the vehicle

The mission document declares the configured execution target. A real
vehicle admits only operational actions. The sequencing core keeps its
own clock for deadlines in both targets.

### The anchor is the launch point

Under `vehicle-estimate` the host converts the vehicle's own fix to NED
from the mission anchor. A fix with an ellipsoidal height is placed
directly. A fix with a declared local-relative height is read as height
above the anchor, so the operator sets the anchor altitude to the launch
point. Other vertical datums have no declared conversion and are refused.
The cruise height stays a height above the anchor.

### Heading

The host takes yaw from a canonical attitude when the vehicle stamps one.
When the vehicle states only a reported Euler attitude, the host takes that
heading. An absent heading stays absent, and the engine commands no
velocity without one.

### Limits

The host tightens the mission ceilings to the velocity envelope the vehicle
advertises before it builds the engine. The engine never commands beyond
the advertised envelope.

## Consequences

- A physical vehicle can fly a planned mission without an independent
  position source. The solution has no independent integrity, and the
  display says so.
- The reported heading's sign and north reference, and the vertical
  velocity sign of a consumer multicopter, need a flight capture before the
  first automatic flight. The adapter refuses a fix outside the Earth, so
  a unit error produces no fix and no flight.
- The executor's pause and resume behavior after a human takeover is a
  separate decision.
