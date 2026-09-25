# ADR-0046: An aircraft domain owns configuration, performance, and fuel

- Status: Proposed
- Date: 2026-09-23

## Context

An agent must answer questions like "Can we reach the alternate with the
fuel or battery energy on board?" An EFB must calculate weight and balance
and takeoff performance. Both need data about the aircraft itself:

- the configuration: type, equipment, limits, and seats and stations,
- the performance model: tables or formulas for takeoff, climb, cruise, and
  landing,
- the energy system: fuel tanks, capacities, and usable fuel, or the battery
  and its usable energy,
- the live state: engine parameters, fuel quantity, fuel flow, and battery
  state.

No domain owns this data. ADR-0018 carries attitude, motion, and estimator
state. It carries no engine, fuel, or battery values. No link that Pilotage
reads decodes engine, fuel, or battery records. No `avionics-link` adapter
exists in Pilotage. The MAVLink adapters do not decode battery records. The
X-Plane path, `crates/pilotage-xplane-trial`, does not decode fuel records.
Nothing owns the static configuration, the live state, or the derived
results.

The vehicles that Pilotage flies in simulation are electric. For them, the
only onboard energy is the battery.

## Decision

### The Aircraft domain

An `Aircraft` domain joins the situational domains of ADR-0036. It follows
the same lifecycle rules: one owner, immutable snapshots, and explicit
validity.

| Part | Lifecycle | Source |
|---|---|---|
| Aircraft profile | Versioned and hashed, like calibration records (ADR-0021) | Operator, manufacturer data, or an installed pack |
| Loading | One record for each flight, with revisions | Operator or mission plan |
| Live engine, fuel, and battery state | Stamped samples with source and clock (ADR-0009) | Planned: an `avionics-link` adapter for engine and fuel, and MAVLink `BATTERY_STATUS` for electric vehicles |
| Derived results | Immutable results that name their inputs | The domain's calculators |

Each derived result names the profile hash and the inputs that it used.

- A weight-and-balance result names the loading revision. It has no validity
  time, because a loading does not expire.
- An endurance result names the energy state that it used and the time of
  that energy state. It carries a validity time, so a consumer can see when
  the result is stale.
- A live sample keeps its source stamp in the telemetry that carries it.

### The profile is data

The aircraft profile is data in a documented schema, not code. A new aircraft
type is a new profile, not a new build. The calculators read the profile.

### The domain does not certify

The calculators are advisory. The domain records the source and revision of
each profile. It makes no airworthiness or certification claim.

## Consequences

- Agents and the EFB read one aircraft state through `SituationView` and the
  agent context port (ADR-0047).
- A mission plan can check fuel or battery energy and performance before
  release against the same profile that the onboard host uses.
- The profile schema, the first calculators, and the `avionics-link` engine
  and fuel adapters are tracked as separate work.
- A live-state adapter needs a link that carries engine, fuel, or battery
  records first. The endurance calculator accepts a measured fuel flow or
  electrical power, but no adapter supplies one. Until an adapter exists, the
  energy on board comes from the loading record: the fuel in each tank, or
  the battery energy at the start of the flight.
- The profile must state whether the aircraft stores fuel or battery energy.
  The calculators must not treat battery energy as litres of fuel.
