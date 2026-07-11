# ADR-0018: Avionics state rides telemetry additively; Aviate joins through a MAVLink adapter

- Status: Accepted
- Date: 2026-07-09

## Context

The telemetry plane today carries planar ground-vehicle state: `Pose2d`
(x, y, heading) and a scalar speed. The instrument runtime (ADR-0017) needs
attitude, altitude, and vertical state, and the first vehicle that produces
them for real is the Aviate flight controller: its Gazebo SITL flies an X500
quadrotor and emits a deliberate MAVLink 2.0 subset — HEARTBEAT,
ATTITUDE_QUATERNION, LOCAL_POSITION_NED — over UDP in SITL and USB CDC on
hardware. Aviate's stated boundary is that it never does UI or GCS work, so
the display side of that contract has to live here.

Two constraints from standing decisions: schema evolution must be additive
(ADR-0014, enforced by `buf breaking`), and vehicles enter through the
`VehicleAdapter` boundary, never through bespoke host paths (ADR-0008).

## Decision

### `AvionicsState` is an additive sub-message of `TelemetrySample`

`pilotage.v1.TelemetrySample` gains an optional `AvionicsState` field carrying
the raw state estimate, not display-ready numbers: attitude quaternion
(w, x, y, z), body angular rates, NED position, NED velocity, a validity
bitmask and quality enum mirroring Aviate's `StateValidFlags` /
`EstimateQuality`, plus independent attitude/rates and kinematics measurement
stamps. Ground vehicles simply never set the field; nothing existing changes
shape.

Each group stamp carries a vehicle-scoped source identity, a source epoch, a
wrapping group sequence, a monotonic acquisition timestamp, and its clock
domain. Re-publishing a cached value preserves the complete stamp. The host's
top-level `observed_at` remains publication/transport metadata and never
relabels source acquisition time.

The receiver accepts a group only when its epoch/sequence advances under
wrap-safe serial arithmetic. Duplicates, reordering, older epochs, other
vehicles, and unselected sources are counted and cannot replace display state
or refresh its age. A newer epoch clears every group from the earlier epoch so
one display generation cannot mix values across a source reset.

Attitude and kinematics retain separate stamps. The ingress gate publishes an
immutable display generation and an explicit coherence result derived only
when source identity, epoch, and clock domain match and acquisition-time skew
meets the selected display profile. Publication in one `AvionicsState` does not
by itself imply coherent acquisition.

The wire stays raw because derivation is display policy: barometric-style
altitude (−z), vertical speed (−vz), groundspeed (√(vx²+vy²)), and
Euler attitude all derive in `pilotage-instrument-state`, where they are unit
tests in a `no_std` crate rather than per-host arithmetic. A host that
forwards the estimate untouched cannot skew it.

### Aviate connects through `adapters/aviate`

A new adapter crate owns the Aviate vehicle end to end:

- **Telemetry**: binds the MAVLink GCS UDP port (default 14550) and parses
  MAVLink v2 frames with a minimal hand-rolled parser — magic `0xFD`, header,
  24-bit message id, CRC-16/MCRF4XX seeded with each message's CRC_EXTRA,
  and v2 payload-truncation zero-extension — for exactly the three message
  ids Aviate emits. Frames that fail CRC or carry unknown ids are counted
  and skipped. The parser is a plain `no_std`-style module with no I/O so
  the frame math is unit-testable byte-for-byte.
- **Mapping**: ATTITUDE_QUATERNION + LOCAL_POSITION_NED fold into the
  vehicle's `TelemetrySample` (planar pose from x/y + yaw for existing
  consumers; full estimate into `AvionicsState`). MAVLink `time_boot_ms` is
  retained for both groups. Independently advancing wrapping sequences reject
  duplicate and reordered measurements before they enter the cache. A
  confirmed boot-clock reset or clock wrap starts a new explicit source epoch.
  Group receive time controls withholding, while display freshness advances
  only with the source stamp, so cached publications age rather than freezing.
- **Control**: the adapter is telemetry-only in this increment. Its
  capabilities advertise no controllable scopes; control frames are rejected
  at the boundary. Command uplink (arm/disarm via COMMAND_LONG through
  Aviate's security gateway) is a later decision, not a hidden TODO in the
  control path.
- **Video**: the Aviate SITL world is ordinary Gazebo, so camera frames reach
  the browser the same way the yard world's do — a camera sensor in the world
  SDF bridged by the existing gz sidecar. The adapter may spawn the sidecar
  for its cameras exactly as the Gazebo adapter does; no new media path.

**Why not the `rust-mavlink` crate:** it generates the entire MAVLink common
dialect (hundreds of messages) to use three, and its message structs would
become a second vocabulary fighting the schema. The subset parser is a few
hundred lines with exhaustive tests, matches Aviate's own
minimal-subset ethos, and keeps the dependency wall around the telemetry
plane. If the message set ever grows past a handful, revisit.

**Why not teach Aviate the Pilotage protocol:** MAVLink is Aviate's public
contract and its hardware transport (USB CDC) already speaks it; coupling the
FC to a display protocol would invert the dependency this ADR exists to keep
one-way.

## Consequences

- `buf breaking` passes by construction; old clients ignore the new field.
- The browser wire decoder and the host fill site each grow one arm; the
  instrument bridge (ADR-0017) is the only consumer that interprets it. The
  decoder preserves uint64 identities/timestamps without narrowing them to an
  imprecise JavaScript number.
- Aviate's current wire gap becomes visible instead of papered over: no
  airspeed or barometric sensor message exists yet, so IAS renders `Missing`
  on the PFD — the honest display is the feature, and the gap is Aviate's to
  fill (its `TelemetryCycleFormatter` is the extension point).
- The same adapter binds any MAVLink v2 source that emits the same three
  messages (PX4 SITL does), which is a free conformance check against a
  second producer, but Aviate remains the contract we track.
- On hardware, the same parser reads the same bytes over USB CDC; only the
  transport binding differs.
