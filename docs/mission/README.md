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
built-in demo route through the fixture snapshot; any other value is a
route string over the store configured by `PILOTAGE_MISSION_NAVDATA` +
`PILOTAGE_MISSION_DATE`). The host logs the pack-for-flight record
(route, cycle, digest, fixture flag) at startup and the mission state
transitions and counters as it flies. A browser joining the session
mid-flight sees automation holding `vehicle.motion`.

Run records in this directory are structured captures of acceptance
flights: the pack-for-flight line, state transitions, terminal counters,
and the gate context they flew under.
