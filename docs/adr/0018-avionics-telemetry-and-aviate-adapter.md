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

Each group stamp carries a vehicle-scoped logical source identity, an opaque
128-bit source incarnation, a source epoch, a wrapping group sequence, a
monotonic acquisition timestamp, and its clock domain. Incarnations are
equality-only attachment or boot tokens; epoch ordering is meaningful only
inside one incarnation. Re-publishing a cached value preserves the complete
stamp. The host's top-level `observed_at` remains publication/transport
metadata and never relabels source acquisition time.

The receiver accepts a group only when its incarnation is authorized and its
epoch/sequence advances under wrap-safe serial arithmetic. Duplicates,
reordering, previously seen incarnations, older epochs, acquisition-time
regressions, other vehicles, and unselected sources are counted and cannot
replace display state or refresh its age. A new incarnation or epoch clears
every group from the earlier identity so one display generation cannot mix
values across a source reset. Aircraft profiles pin a source-issued
incarnation during authenticated bootstrap. The browser simulator profile may
accept a bounded number of unseen incarnations and resets all ingress history
after each newly negotiated WebTransport session; that policy is explicitly
ineligible for operational credit.

Attitude and kinematics retain separate stamps. The ingress gate publishes an
immutable display generation and an explicit coherence result derived only
when source identity, epoch, and clock domain match and acquisition-time skew
meets the selected display profile. Publication in one `AvionicsState` does not
by itself imply coherent acquisition.

Group presence is structural throughout the adapter API. Missing attitude or
kinematics is represented as `None`, never an identity quaternion, origin, or
zero velocity. The planar `pose` and `velocity` messages are emitted only when
both source groups share identity/clock and meet the selected skew bound; a
single or incoherent group continues to flow independently through
`AvionicsState`. Receivers treat absent message fields and group stamps as
missing data.

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
  and skipped. Every accepted frame must match the configured MAVLink system
  and component; the logical source is configured rather than selected by the
  first estimate. The parser is a plain `no_std`-style module with no I/O so
  the frame math is unit-testable byte-for-byte.
- **Mapping**: ATTITUDE_QUATERNION + LOCAL_POSITION_NED fold into the
  vehicle's `TelemetrySample` (a planar projection only when both groups are
  available and coherent within the selected skew bound; each raw group
  independently into `AvionicsState`). MAVLink `time_boot_ms` is retained for
  both groups. Independently advancing wrapping sequences reject
  duplicate and reordered measurements before they enter the cache. A 32-bit
  boot-clock wrap starts a new explicit source epoch. Ordinary MAVLink has no
  trustworthy boot UUID, so the default policy never infers a reboot from a
  replayable clock regression. The simulator policy first requires accepted
  measurement silence, then same-group source-time and receive-time dwell; a
  different group cannot confirm the transition. Its 300 ms inter-group
  high-water allowance is a simulator profile input, not an aircraft value.
  Group receive time controls withholding, while display freshness advances
  only with the source stamp, so cached publications age rather than freezing.
- **Shared-memory mapping**: the simulator reader verifies the exact 216-byte
  ABI and records the POSIX object's `(device, inode, size)` identity before
  mapping. Reopening the same frozen object cannot change epoch or freshness.
  A different object plus a coherent first sample authorizes an attachment
  transition; sequence or acquisition-time rollback on the same object is
  quarantined. The current Aviate SHM ABI has no writer incarnation field, so
  inode identity remains simulator-only. An aircraft-capable producer must
  provide a source-issued boot identity or persistent monotonic boot counter.
- **Control**: when the command uplink is available, the adapter advertises the
  flight-control scope and addresses setpoints to the same configured MAVLink
  system and component used by telemetry. An unavailable uplink degrades to
  telemetry-only capability rather than changing the measurement source.
  Control requiring a current pose is rejected when either source group is
  missing, stale, identity-incompatible, or outside the configured skew bound;
  the adapter never seeds a setpoint with zero substitutes. Disarm is a
  measurement-independent safety action and remains available when the
  estimate is unavailable; a later arm still requires a coherent measured yaw.
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
  imprecise JavaScript number and preserves the 128-bit incarnation exactly.
- Aviate's current wire gap becomes visible instead of papered over: no
  airspeed or barometric sensor message exists yet, so IAS renders `Missing`
  on the PFD — the honest display is the feature, and the gap is Aviate's to
  fill (its `TelemetryCycleFormatter` is the extension point).
- The same adapter binds any MAVLink v2 source that emits the same three
  messages (PX4 SITL does), which is a free conformance check against a
  second producer, but Aviate remains the contract we track.
- On hardware, the same parser reads the same bytes over USB CDC; only the
  transport binding differs. A hardware binding must inject its source-issued
  incarnation rather than use the simulator operating-system entropy provider.
