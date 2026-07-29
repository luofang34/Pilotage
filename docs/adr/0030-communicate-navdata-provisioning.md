# ADR-0030: The host consumes aeronautical navdata through Communicate's cycle-dated snapshot surface

- Status: Proposed
- Date: 2026-07-29

## Context

Mission execution ([ADR-0025](0025-client-optional-operation-automation-principals.md))
and the moving-map/EFB path ([ADR-0023](0023-vehicle-side-decomposition-fc-navigate-communicate.md),
[ADR-0026](0026-host-capability-profiles.md)) need aeronautical entities —
airports, fixes, navaids, airways, airspace — with known effectivity.
Communicate already ships exactly that: NASR/CIFP data packed into a
versioned, checksummed blob format with 28-day cycle selection, a
provenance-tracked manifest/sync distribution chain, identifier
resolution, and route expansion, all available as a device-clean library
slice with no network or runtime dependencies.

Terrain is adjacent but different: `pilotage-svs-db` packages are
self-authenticating (Ed25519 signature plus Merkle tile root verified
against a host-held trust root), the build chain processes only synthetic
in-memory sources, and no decodable container format exists to move a
package between machines. Communicate's manifest/sync chain is
content-blind in transport but its store/selection layer decodes its own
blob format to select cycles.

## Decision

- **Navdata enters the host only through Communicate's snapshot
  surface**: cycle-dated snapshots decoded from its versioned blob
  format, selected by effectivity date. No other ingestion path (direct
  NASR/CIFP parsing, ad-hoc waypoint files) exists on the host.
- **Provenance travels with use.** A plan built from navdata records its
  source: route input, authority, cycle effectivity, and blob digest —
  the auditable "pack for flight" record. Fixture snapshots for
  simulation traverse the same encode/decode path and are marked as
  fixtures in that record.
- **Advisory discipline holds** ([ADR-0023](0023-vehicle-side-decomposition-fc-navigate-communicate.md)):
  navdata is advisory context. It shapes plans, guidance targets, and
  displays; it never enters the telemetry plane as measurement and never
  gates the authority machinery.
- **Units convert exactly once.** Communicate's vocabulary (decimal
  degrees, nautical miles) converts to the canonical radians/meters
  vocabulary at the host's Navigate binding — one named boundary, no
  mixed-unit arithmetic anywhere else.
- **Currency gates mission start, not mission continuation.** Starting a
  mission requires a snapshot effective for the operation date; an
  explicit, logged override exists for simulation and development. A
  cycle rolling over in flight never invalidates the active plan.
- **Terrain distribution is deferred with its design recorded.**
  Communicate's manifest/sync chain is the intended discovery and
  transport for terrain packages, carrying them as opaque blobs whose
  trust anchor stays the package's own signature against the host's
  trust root — the manifest checksum is transport integrity, never
  content authority. Four prerequisites gate implementation:
  1. real DEM/obstacle ingestion in the terrain build chain (only
     synthetic in-memory sources exist);
  2. a decodable package container format (encoding exists for
     reproducibility comparison only);
  3. a format-agnostic binding in Communicate's store/selection layer
     (which requires its own blob format for cycle selection);
  4. host-side staged handoff from the synced store into verify-and-
     activate under the host trust root.
  Terrain distributed this way earns SVS and moving-map display use
  only. Terrain-awareness credit (EGPWS-class) belongs to Navigate's own
  qualification ([ADR-0024](0024-navigation-authority-boundary.md)) and
  is never implied by distribution.

## Consequences

- Simulation and tests fabricate small snapshots and push them through
  the real blob encode/decode path — the consumer code under test is
  identical to the live-sync path, with no network.
- The EFB client's embedded posture ([ADR-0026](0026-host-capability-profiles.md))
  consumes the same library slice; host and client read one navdata
  truth.
- Route expansion failures (unresolved identifier, unknown airway) are
  typed refusals carrying the snapshot's cycle — a stale store is
  diagnosable from the error alone.
- Communicate remains free of operational authority: the host decides
  what a mission may do with the data; Communicate only vouches for what
  the data is and where it came from.

## Alternatives considered

- **Host-side NASR/CIFP ingestion:** rejected; it duplicates a mature,
  tested pack/verify/select chain and splits provenance across repos.
- **Wrapping terrain packages in Communicate's blob format:** rejected;
  double-wrapping obscures the package's own signature, and the
  manifest-as-transport split (checksum for transport, signature for
  content) keeps trust anchored where verification happens.
