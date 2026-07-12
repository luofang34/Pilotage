# ADR-0018: Avionics state rides telemetry additively; Aviate joins through a MAVLink adapter

- Status: Accepted
- Date: 2026-07-09

## Context

The telemetry plane today carries planar ground-vehicle state: `Pose2d`
(x, y, heading) and a scalar speed. The instrument runtime (ADR-0017) needs
attitude, altitude, and vertical state, and the first vehicle that produces
them for real is the Aviate flight controller: its Gazebo SITL flies an X500
quadrotor and emits a deliberate MAVLink 2.0 subset — HEARTBEAT,
ESTIMATOR_STATUS, AVIATE_ESTIMATOR_STATUS, ATTITUDE_QUATERNION, and
LOCAL_POSITION_NED — over UDP in SITL and USB CDC on hardware. Aviate's stated
boundary is that it never does UI or GCS work, so the display side of that
contract has to live here.

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
stamps and an independent estimator-status stamp. Ground vehicles simply never
set the field; nothing existing changes shape.

Each group stamp carries a vehicle-scoped logical source identity, an opaque
128-bit source incarnation, a source epoch, a wrapping group sequence, a
monotonic acquisition timestamp, and its clock domain. Incarnations are
equality-only attachment or boot tokens; epoch ordering is meaningful only
inside one incarnation. Re-publishing a cached value preserves the complete
stamp. The host's top-level `observed_at` remains publication/transport
metadata and never relabels source acquisition time.

The receiver accepts a group only when its incarnation is authorized and its
epoch/sequence advances under wrap-safe serial arithmetic. Duplicates,
reordering, already-seen incarnations, older epochs, acquisition-time
regressions, other vehicles, and unselected sources are counted and cannot
replace display state or refresh its age. A new incarnation or epoch clears
every group from the earlier identity so one display generation cannot mix
values across a source reset. Aircraft profiles pin a source-issued
incarnation during authenticated bootstrap. The browser simulator profile may
accept a bounded number of unseen incarnations and resets all ingress history
after each newly negotiated WebTransport session; that policy is explicitly
ineligible for operational credit.

Attitude, kinematics, and estimator status retain separate stamps. The ingress
gate publishes an immutable display generation and an explicit coherence result
derived only when source identity, epoch, and clock domain match and
acquisition-time skew meets the selected display profile. Publication in one
`AvionicsState` does not by itself imply coherent acquisition.

`AVIATE_ESTIMATOR_STATUS` is the lossless authorization source. Its validity
bits and quality authorize a numeric group only when the FC acquisition
timestamps match exactly. Missing, malformed, reordered, timestamp-mismatched,
or unknown authorization fails closed. Each accepted status can only retain or
downgrade the authorization latched onto an already cached numeric group; a
later Good status cannot restore data from another timestamp. A new exact-paired
numeric group establishes a new authorization baseline. Aviate emits the
status pair immediately before every numeric snapshot and also permits
status-only cycles, so revocation never waits for another numeric message. The common
`ESTIMATOR_STATUS` projection is decoded for diagnostics but never grants
authorization because it cannot represent Aviate's per-signal contract without
loss.

A private-status CRC or structural parse failure immediately downgrades every
cached numeric group to Unusable. Because an invalid frame cannot supply a
trustworthy source acquisition time, this local downgrade retains the last
valid status stamp. The parser preserves the failed frame's header source and
position as a revoke-only event, so source selection still applies and a later
failure in a multi-frame datagram wins. Receivers admit a duplicate-stamped
effective authorization only when it monotonically removes validity or worsens
quality; it advances the display generation without refreshing any source-group
age. Duplicate-stamped authorization can never restore data.

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
  and v2 payload-truncation zero-extension — for the focused Aviate message
  contract. Frames that fail CRC or carry unknown ids are counted and skipped.
  Every accepted frame must match the configured MAVLink system and component;
  the logical source is configured rather than selected by the first estimate.
  The parser is a plain `no_std`-style module with no I/O so the frame math is
  unit-testable byte-for-byte against producer-owned golden vectors.
- **Mapping**: AVIATE_ESTIMATOR_STATUS authorizes ATTITUDE_QUATERNION and
  LOCAL_POSITION_NED only through the exact timestamp and monotonic-latching
  rules above. The numeric messages fold into the
  vehicle's `TelemetrySample` (a planar projection only when both groups are
  available and coherent within the selected skew bound; each raw group
  independently into `AvionicsState`). MAVLink boot time is retained for all
  three acquisition groups. Independently advancing wrapping sequences reject
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
dialect while Aviate also carries a small private status message, and its
message structs would become a second vocabulary fighting the schema. The
focused parser has exhaustive producer-vector tests, matches Aviate's own
minimal-subset ethos, and keeps the dependency wall around the telemetry plane.
If the message contract grows substantially, revisit.

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
- Numeric messages from another MAVLink source remain observable but fail
  closed unless an explicit adapter mapping supplies equally lossless
  authorization. A common ESTIMATOR_STATUS projection alone is insufficient.
- AVIATE_ESTIMATOR_STATUS belongs to a private dialect whose producer contract
  is not yet declared stable. Producer-owned golden vectors make any layout or
  CRC drift fail tests, but an operational integration also requires a declared
  stable dialect and an assigned message-id range.
- On hardware, the same parser reads the same bytes over USB CDC; only the
  transport binding differs. A hardware binding must inject its source-issued
  incarnation rather than use the simulator operating-system entropy provider.
