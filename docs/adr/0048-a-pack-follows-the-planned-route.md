# ADR-0048: A pack follows the planned route and is checked before flight

- Status: Proposed
- Date: 2026-09-23

## Context

An operator must prepare all the data for a flight in one step: select the
route or the area, and download everything. The data includes navigation data
(NASR and CIFP), charts, terrain, obstacles, imagery, and visual reference
features. Visual reference features are map data. `navigate-visual` makes
reference views from them and matches camera frames against those views
(Navigate ADR-0007). The same data must be available on the web
client, the iPad, and the onboard host.

These parts exist or are proposed:

- signed catalogs, releases, and installation, from the offline aviation
  data work,
- route and area coverage planning (`navigate-imagery`),
- content-addressed chunks and package lineage (Navigate),
- one logical store with storage classes (ADR-0044).

There is no object for "the data for this flight".

## Decision

### The pack specification

A `PackSpec` names:

- the coverage: a flight plan revision, a route with a corridor width, or an
  area,
- the products and, for each product, the release that the pack uses,
- the validity window of the flight.

Some products have national coverage: the pack includes the whole release,
for example a NASR cycle. Other products are tiled: the pack includes the
tiles inside the corridor, for example terrain, obstacles, imagery, charts,
and visual reference features.

Resolving a spec gives a `PackPlan`: the chunks for each product, the total
size, and the size that is not installed yet. The operator sees the size
before the download starts.

### The plan link and the preflight check

A pack made from a flight plan revision names that revision. Before release,
the host checks that each product the mission needs is installed, covers the
route, and is current for the flight time. The result is a record in the
Briefing domain. Agents (ADR-0047) and the EFB read the same result.

### Currency

Each release has a validity period. The period is 28 days for NASR and CIFP
data. The period is 56 days for charts. A pack shows its state: current, expiring, or expired. An update
downloads only the changed chunks, because releases share chunks.

### Delivery between devices

The EFB can download a pack on a ground network and deliver it to the onboard
host over the Bulk configuration message class (ADR-0011). The host requests only the
chunks it does not have. The transfer can resume after an interruption,
because each chunk is identified by its digest.

### Platform limits

- The web client compares the pack size with the storage quota before the
  download. When the pack does not fit, the web client offers a lower
  imagery zoom.
- Packs use the `Offline` storage class (ADR-0044). On Apple platforms, the
  Apple host excludes the `Offline` root from backup.
- Licensed products require an entitlement (ADR-0027).

### Performance

- Resolving a spec does not download data. It reads catalogs and tile
  indexes.
- Downloads run in parallel by chunk, within a bandwidth and concurrency
  limit.
- A chunk is verified once, when it is written. Readers trust the store and
  the manifest (ADR-0044).

## Consequences

- One pack serves the web client, the iPad, and the onboard host.
- A mission cannot be released without evidence that its data is installed
  and current.
- The `PackSpec` resolver, the preflight check, and pack delivery over the
  session are tracked as separate work.
