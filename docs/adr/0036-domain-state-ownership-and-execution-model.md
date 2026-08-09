# ADR-0036: Domain-state ownership by data category, and the execution model that connects it

- Status: Proposed
- Date: 2026-08-09
- Supersedes on acceptance: [ADR-0035](0035-source-neutral-situational-services.md)

## Context

[ADR-0035](0035-source-neutral-situational-services.md) gave AeroContext the
weather, NOTAM, TFR, briefing, and navigation data. That boundary was correct
for early work. It is now too wide.

Two facts show the problem.

First, the state rules are not the same. Weather data is perishable. It has no
baseline. Navigation data has a cycle-dated baseline. Notices and restrictions
amend that baseline. Evidence: the `cycle` concept occurs in `navdata.rs`,
`freshness.rs`, and `provenance.rs` in `aerocontext-core`. It occurs zero times
in `weather.rs` and `metar.rs`.

Second, a 978 MHz ground uplink is not a weather stream. It carries weather
products, aeronautical notices, and link-service reports in one stream.
`aero-link` issue #33 identified the products. A router must send each product
to its owner.

ADR-0035 also stated queue rules as principles. It did not state a policy for
each data class. An output consumer that stops a receiver is a performance
failure, so the policy must be explicit.

[ADR-0002](0002-cargo-workspace-portable-sans-io-core.md) requires portable
sans-IO cores and puts I/O in platform ports. This record states how that rule
applies to the aviation services and where asynchronous execution belongs.

Repository topology is not a subject of this record. A package boundary, a
process boundary, and a repository boundary are separate choices.

## Decision 1 — one owner for each data category

| Component | Owns | Does not own |
|---|---|---|
| `aero-link` | Radio access, demodulation, framing, correction, stateless protocol decode, bounded FIS-B segment assembly, and `ReceptionEvent` | Product lifecycle, traffic tracks, or application routing |
| `avionics-link` | Installed-avionics transport, protocol decode, and thin source adapters | A second traffic, weather, or navigation state engine |
| Surveillance | Source-neutral traffic observations, identity, fusion, freshness, tracks, deltas, and snapshots | Radio access, weather, notices, or maps |
| Airmass | Meteorological state: observations, forecasts, and weather hazards. Product revision, validity, expiry, and source comparison | Notices, restrictions, navigation baseline, or maps |
| Airspace | Aeronautical notice and restriction state: NOTAM, TFR, SUA, TRA, TMOA, and MOA status | Meteorological state, the navigation baseline, or maps |
| Navdata | Cycle-dated baseline from NASR and CIFP: airports, navaids, fixes, airways, and procedures | Perishable product state or notice state |
| FlightPlanning | Plan drafts, resolution, validation, and filing state | Plan execution or guidance |
| Briefing | Point-in-time composition of other components' outputs, and the immutable result | Any live domain state of its own |
| Navigate | Ownship navigation fusion, integrity, plan execution, and guidance | Flight-control actuation or advisory product state |
| Aviate or another flight controller | Control-grade estimation, stabilization, command results, and actuation | Advisory product state |
| Pilotage | Composition, capability profiles, sessions, authority, leases, fencing, audit, and read-only situation queries | A second mutable state engine for another domain |
| Indicate | Instrument state contracts, panel sets, scene contracts, registry, and admission tests | Pilotage sessions, maps, or platform UI policy |

`AeroContext` continues as a compatibility facade and as the provider
orchestration point. It must not keep a second copy of the state that Airmass,
Airspace, or Navdata owns. Remove the facade after the last consumer moves.

Briefing composes a result from a plan revision, a requested time, a weather
snapshot, an airspace snapshot, a navigation-data cycle, and provider
documents. Briefing can keep an immutable result for evidence. Briefing must
not keep live domain state.

`SituationView` replaces the joint query interface of AeroContext. It does not
replace data collection, decoding, storage, or planning. `SituationView` is
read-only. It does not own a second copy of any state. Each query states its
valid time, its source, its quality, and its data age.

### Notices are not all airspace

A notice can apply to an aerodrome, a runway, a taxiway, a navaid, a procedure,
an airway, an obstacle, or a communication service. Many notices have no valid
horizontal geometry. Airspace must keep the subject of a notice and must make
its geometry optional. Do not convert every notice into a polygon.

Airspace holds operational airspace state and aeronautical notice state. If
aerodrome notices become large, move them to a separate owner. Do not add that
owner now.

## Decision 2 — one FIS-B assembly mechanism, then a product router

A 978 MHz uplink serves more than one category. Assemble each product one time.
Route each complete product to its owner.

```text
aero-link-core          one-APDU decode, no state
        |
        v
aero-link FIS-B assembly     bounded segment store only
        |
        v
CompleteFisBProduct
        |
        v
product router
        |-- weather products  --> Airmass
        |-- notices and restrictions --> Airspace
        |-- link-service reports --> diagnostics and recording
```

The assembler keeps the product file identity, the segment index and count, a
bounded store of incomplete files, an assembly timeout, and counters for
malformed or conflicting segments. The assembler must not perform product
replacement, expiry, source reconciliation, notice cancellation, or mosaic
composition.

The routing table is a specification. `aero-link` issue #33 supplies it. Write
the table down before either domain contract. Test the table against a recorded
capture.

Product 8 carries notices and also carries FIS-B product-unavailable reports.
Send a product-unavailable report to diagnostics, not to Airspace.

This decision permits bounded, stateful segment assembly inside `aero-link`.
That is a change to the `aero-link` boundary. Segment assembly is a link-layer
mechanism. Product policy stays outside.

## Decision 3 — three execution regimes

[ADR-0002](0002-cargo-workspace-portable-sans-io-core.md) requires sans-IO
cores. Asynchronous execution is correct between components and wrong inside a
sample loop.

| Regime | Rule | Reason |
|---|---|---|
| Signal and frame path | Synchronous. No `await` between a sample buffer and a decoded frame | A 1090 MHz input delivers about 293 transfers each second at 4.8 MB/s. A task switch for each buffer adds latency and copies to the path with the least margin |
| Domain core | Synchronous, single writer, time supplied as data | A core that reads a clock is not deterministic and cannot replay |
| Composition | Asynchronous tasks between components. Bounded channels carry typed values | A slow consumer must not stop a receiver |

A domain core must accept `now` as a parameter. A domain core must not contain
a channel, a task, a lock, or a timer.

An asynchronous provider trait is a seam for network input. Keep such a trait
in an adapter-contract package. Do not keep it in a portable domain core,
because it puts an executor requirement on a package that an embedded build can
otherwise use.

`aero-link-core`, `aero-link-rx`, and `surveillance-core` build for bare-metal
targets. The FIS-B assembler must also build for those targets. A package that
fetches from a network is a host package and does not have this requirement.

## Decision 4 — each channel states its policy

A channel between two components must state four things: its bound, its
backpressure policy, the owner of a dropped item, and the counter that records
the drop.

| Data class | Bound | Policy | Reason |
|---|---|---|---|
| Reception events from a radio adapter | Bounded | Drop oldest, count the drop | Reception must continue. A late sample has no value |
| Track deltas to a display | Bounded, small | Replace the pending state for the same track | A display needs current state, not each intermediate state |
| FIS-B segments during assembly | Bounded store | Keep until complete or until timeout | A partial product has no value, and an unbounded store is a memory fault |
| Complete products to a domain store | Bounded | Block the router briefly, then drop and count | A product arrives at a low rate. A short block is cheaper than a lost product |
| Domain events to `SituationView` | Bounded | Latest snapshot replaces an older pending snapshot | A query reads current state |
| Raw recording | Bounded, independent path | Drop and count | Disk work must not stop reception |

No component may use an unbounded channel between components. Every drop
increments a counter that a consumer can read.

## Consequences

- A new source adds an adapter. It does not change a domain core.
- One assembler serves both product categories. Neither domain repeats it.
- The `AeroContext` facade keeps present consumers working during the move.
- Each component can have its own test corpus, replay tool, resource limits,
  and assurance evidence.
- A headless build links only the components that its capability profile needs.
- A repository split becomes possible later. This record does not require one.
- `aero-link` accepts one bounded stateful component. Its documentation must
  state that exception.

## Migration order

1. Write the FIS-B product routing table and the `CompleteFisBProduct` contract.
2. Define the read-only `SituationView` contract. Let it wrap AeroContext first.
3. Move the asynchronous provider traits out of `aerocontext-core`.
4. Separate Airmass and Airspace state. Keep the AeroContext facade.
5. Make Navdata an explicit boundary.
6. Separate FlightPlanning from provider orchestration.
7. Divide the Leidos response one time into a bundle with one provenance record.
8. Move each consumer to `SituationView`.
9. Remove the AeroContext facade after the last consumer moves.

Step 2 has the highest value and the lowest risk. It fixes the consumer
interface before the state moves, so a consumer moves one time.

## Alternatives considered

- **Keep one AeroContext for all context data.** Rejected. A cycle-dated
  baseline and a perishable product need different state rules, and one
  component with both rules is difficult to test and to assure.
- **Give each category its own FIS-B assembler.** Rejected. One uplink stream
  serves both categories. Two assemblers duplicate a mechanism and can
  disagree.
- **Put asynchronous execution in the domain cores.** Rejected. It removes
  determinism, prevents replay, and blocks embedded reuse.
- **Rename AeroContext to `SituationView`.** Rejected. `SituationView` answers
  queries. It does not collect, decode, or store.
- **Decide repository topology in this record.** Rejected. A repository
  boundary is a governance choice. Contracts and dependency direction are the
  subject here.
