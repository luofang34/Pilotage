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

The host requires an authorized canonical attitude with the same
acquisition identity as the fix. A reported Euler attitude has no verified
north reference and cannot supply mission heading. Without that attitude,
the host refuses the estimate, and the engine commands no
velocity without one.

### Limits

The host tightens the mission ceilings to the velocity envelope the vehicle
advertises before it builds the engine. The engine never commands beyond
the advertised envelope.

### Held and resume

A human interrupts a mission through the handover vocabulary of
[ADR-0010](0010-authority-state-machines.md); no new message exists for
it. A client whose lease request finds the mission as holder asks for a
transfer. The mission principal offers the scope to the requester at once:
human takeover wins ([ADR-0025](0025-client-optional-operation-automation-principals.md)).
When the transfer commits, when the lease is revoked, or when an emergency
override displaces the principal, the mission is **held**: the principal
stops framing, the engine commands nothing, keeps its ownship fresh, and
pauses its sequencing clock so phase deadlines do not count the time a
human flew. The mission wall deadline keeps counting.

The human hands the scope back by offering it to the mission principal.
The principal accepts the offer. On the committed transfer the mission
resumes from its active leg under the new generation. The first frames
under the new generation are neutral, which a vehicle that requires neutral
input on a new lease admits. Accepting an offer executes a human decision;
the mission never requests the scope on its own after a hold.

A human who releases the scope instead of offering it leaves the mission
held: the scope is unassigned and nothing resumes it. To resume, the human
requests the lease and offers the scope to the mission. The same recovery
applies after an operator link loss. Only a closed connection ends the
principal.

### Launch

A simulator's vehicle is armed by the mission. A real vehicle is armed and
launched by its pilot, so a real-vehicle mission has no arm phase and its
first phase is the climb. The mission principal does not request the motion
lease while the vehicle is on the ground, where the vehicle refuses velocity
control: after its welcome it holds, with the phase clocks stopped, until the
vehicle's estimate reports at least 2 m above the launch point. Then it
requests the lease and, once granted, resumes with one neutral frame and
climbs to the survey height. The pilot keeps the original remote controller
and can take the aircraft at any time; the hold and resume rules above apply.

## Consequences

- A physical vehicle can fly a planned mission without an independent
  position source. The solution has no independent integrity, and the
  display says so.
- The reported heading's sign and north reference, and the vertical
  velocity sign of a consumer multicopter, need a flight capture before the
  first automatic flight. The adapter refuses a fix outside the Earth, so
  a unit error produces no fix and no flight.
- A takeover and a hand-back use the existing handover protocol, so every
  client that can request a transfer and offer a scope can interrupt and
  resume a mission. A client that only requests and releases leases can
  interrupt but not resume.
