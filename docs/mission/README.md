# Mission executor — demo runs

The in-process mission executor ([ADR-0025](../adr/0025-client-optional-operation-automation-principals.md))
flies a route built from Communicate-provisioned navdata
([ADR-0030](../adr/0030-communicate-navdata-provisioning.md)) through the
session host's fenced authority path: it attaches as an automation-class
local principal (hello → welcome → profile activation → lease → typed
arm → typed velocity intents), with the same fencing, watchdog, and
rejection accounting as any remote client. Navigate supplies fusion,
plan sequencing, and velocity guidance; the FC never sees anything but
ordinary adapter setpoints.

Run it against a live SITL session:

```sh
PILOTAGE_MISSION_ROUTE=fixture cargo xtask sim --fc aviate-gz
```

`PILOTAGE_MISSION_ROUTE` enables the executor (`fixture` selects the
built-in demo route). Any other value is a route string expanded against
the configured navdata: by default the fixture snapshot (so custom
routes must name fixture idents), or a real synced store when
`PILOTAGE_MISSION_NAVDATA` points at one, with `PILOTAGE_MISSION_DATE`
selecting the cycle. The host logs the pack-for-flight record (route,
cycle, digest, fixture flag) at startup and the mission state
transitions and counters as it flies. A browser joining the session
mid-flight sees automation holding `vehicle.motion`.

Terminal behavior: after the final waypoint captures (`PlanComplete`)
the executor holds a zero-velocity hover — the adapters' brake-then-hold
takes over — with the vehicle armed and the motion lease held
indefinitely, and navigation guidance goes absent (the HSI removes the
needle). Landing, disarm, and lease-release policy at mission end is
part of the failure-detection scope tracked in issue #245.

Run records in this directory are structured captures of acceptance
flights: the pack-for-flight line, state transitions, terminal counters,
and the gate context they flew under.
