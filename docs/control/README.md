# Typed control-authority slice (INPUT-01 / CTRL-01)

SIM / NOT FOR FLIGHT. Engineering trace record for the typed browser
control plane: profile-bound frames, reliable discrete actions, exclusive
authority groups, input-source identity, and the simulator lifecycle
capability. This document holds the requirement statements the evidence
graph (`evidence-graph.evg`) traces; it establishes no certification claim.

## Intended function

The browser control plane commands a simulated vehicle exclusively through
typed, capability-negotiated intents and actions, each attributable to the
exact input mapping and authority epoch that produced it.

## Hazard

**CTRL-HAZ-01 — unattributable or replayed control authority.** A control
command (setpoint or discrete action) that executes under an authority
epoch, input mapping, or channel it was not bound to: a delayed ARM
re-arming after a DISARM, a frame produced by an unannounced mapping, a
sibling scope escaping a link-loss brake, keyboard input attributed to a
gamepad profile, or a simulation reset fired from flight authority.

## Requirements

### CTRL-BIND-001 {#ctrl-bind-001}

Typed control binds to the announced composite profile activation. The
activation announcement names the scheme AND device documents (identity,
revision, SHA-256 content digest) under a monotonic activation revision
validated against the sender's own session; a typed frame whose activation
revision does not match the announced record is rejected before the
command gate, and evidence records only accepted announcements.

### CTRL-CHAN-002 {#ctrl-chan-002}

Discrete actions ride only the reliable ordered session stream. Each
command carries its full authority binding — session, vehicle, scope,
fencing generation, announced activation revision — and a required nonzero
correlation id; the host validates every binding against its own records,
answers every command with a correlated result on the same stream,
deduplicates by id plus the immutable request fingerprint, and refuses a
reused id carrying different content. Anti-replay is watermarked, not
merely cached: an id at or behind the newest admitted one is permanently
stale — refused, never executed — even after its result leaves the
bounded cache, across the full u32 wrap. Every valid command yields
exactly one correlated result, including when the adapter rejects the
frame before per-action disposal. A datagram frame carrying typed actions
is rejected whole. A delayed or replayed press bound to a superseded
generation is refused, never executed. Acceptance of a profile activation
is an explicit engine event; evidence derives from that event alone.

### CTRL-GROUP-003 {#ctrl-group-003}

Scopes driving one actuator form an exclusive authority group, and the
lease binds the CONCRETE member scope it was acquired for. Group members
are never held simultaneously (by anyone); leases, holder identity,
fencing generations, the frame-silence watchdog, the link-loss latch, and
neutral recovery all operate on the group — but the holder commands only
the member it leased: a frame or action on the unleased sibling is a
non-holder's, even under the shared generation. Switching members goes
release-first — neutralize, re-fence — and the sibling grant lands
strictly newer in the one generation domain, never leaving an orphaned
sibling latch.

### INPUT-SRC-004 {#input-src-004}

The activation announcement names the input source actually driving. The
keyboard is a layered registry profile with its own identity, revision,
and digest; pad connect, pad disconnect, and same-model replacement each
switch the active source through the transactional neutral handover and
re-announce the new source's real identity.

### CTRL-LIFE-005 {#ctrl-life-005}

Simulation reset is a separately authorized, simulator-only lifecycle
capability, structural to the adapter's PROFILE. `SIM_RESET` is advertised
only on the `sim.lifecycle` scope of simulation-profile adapters — never
on a flight scope, never in a legacy flight mapping, and never on a
physical/RF profile, which neither advertises nor executes lifecycle
commands regardless of its uplink — and commanding it requires that
scope's own lease.
