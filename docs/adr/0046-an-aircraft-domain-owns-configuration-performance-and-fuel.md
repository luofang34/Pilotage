# ADR-0046: An aircraft domain owns configuration, performance, and fuel

- Status: Proposed
- Date: 2026-09-23

## Context

An agent must answer questions like "Can we reach the alternate with the
fuel on board?" An EFB must calculate weight and balance and takeoff
performance. Both need data about the aircraft itself:

- the configuration: type, equipment, limits, and seats and stations,
- the performance model: tables or formulas for takeoff, climb, cruise, and
  landing,
- the fuel system: tanks, capacities, and usable fuel,
- the live state: engine parameters, fuel quantity, and fuel flow.

No domain owns this data. ADR-0018 carries avionics telemetry, which contains
some live engine values. Nothing owns the static configuration or the derived
results.

## Decision

### The Aircraft domain

An `Aircraft` domain joins the situational domains of ADR-0036. It follows
the same lifecycle rules: one owner, immutable snapshots, and explicit
validity.

| Part | Lifecycle | Source |
|---|---|---|
| Aircraft profile | Versioned and hashed, like calibration records (ADR-0021) | Operator, manufacturer data, or an installed pack |
| Loading | One record for each flight, with revisions | Operator or mission plan |
| Live engine and fuel state | Stamped samples with source and clock (ADR-0009) | `avionics-link` adapters |
| Derived results | Immutable results that name their inputs | The domain's calculators |

A derived result, such as endurance, range, or a weight-and-balance check,
names the profile hash, the loading revision, and the live samples that it
used. It carries a validity time. A consumer can see when a result is stale.

### The profile is data

The aircraft profile is data in a documented schema, not code. A new aircraft
type is a new profile, not a new build. The calculators read the profile.

### The domain does not certify

The calculators are advisory. The domain records the source and revision of
each profile. It makes no airworthiness or certification claim.

## Consequences

- Agents and the EFB read one aircraft state through `SituationView` and the
  agent context port (ADR-0047).
- A mission plan can check fuel and performance before release against the
  same profile that the onboard host uses.
- The profile schema, the first calculators, and the `avionics-link` engine
  and fuel adapters are tracked as separate work.
