# LINK-04 source-role slice — engineering trace record

SIM / NOT FOR FLIGHT. This directory holds the LINK-04 vertical slice of
the lifecycle evidence graph: source-role separation between the FC
operational estimate, simulation truth, and FC-owned vehicle state. It
establishes no certification claim; see `evidence-graph.evg` for the
maintained trace and `evidence-artifacts/` for recorded runs.

## Intended function {#link-if}

The vehicle-link telemetry plane presents the FC operational estimate to
primary panels and operational control, while a simulation profile may
additionally expose simulator ground truth as a clearly-roled oracle for
logging, test assertions, and estimate-versus-truth comparison.

## Hazard {#link-haz-01}

**LINK-HAZ-01 — truth-as-estimate masquerade.** Simulator ground truth
silently drives primary panels or command construction in place of the
FC operational estimate. SITL behavior then diverges from a real
vehicle, estimator and link faults are masked, and control decisions are
seeded from data no physical deployment will have.

## Requirements

### LINK-ROLE-001 {#link-role-001}

Every measurement carries an explicit source role and integrity
classification in its provenance stamp; estimate, simulation-truth, and
FC-state observations are structurally separate samples with independent
identity, epoch, sequence, clock, availability, and integrity. Consumers
gate on the exact role — primary panels admit only the operational
estimate, and mislabeled or unstamped lanes are unconsumable.

### LINK-CTRL-002 {#link-ctrl-002}

Operational command construction requires a live, authorized FC
operational estimate whose stamps carry the operational-estimate role.
Simulation truth can never seed a command; loss or mislabeling of the
estimate rejects or neutralizes state-dependent control even while truth
remains healthy, and an oracle-only session advertises no motion-control
scope at all.

### LINK-PROV-003 {#link-prov-003}

FC-owned vehicle state is published under its own stamp carrying the
configured FC identity, host-receive clock, and checksummed-only
integrity; role, identity, and integrity survive the host-to-client wire
and session capture verbatim, and duplicate reports never refresh
freshness downstream.

### LINK-AUTHZ-005 {#link-authz-005}

Client-side authorization of an estimate group is decided by the
estimator-status regime that governed that group's acquisition instant,
bounded by the coherence budget — so a numeric group interleaving with a
faster status stream keeps the authorization it was actually granted
instead of being stripped whenever the two lanes arrive a few
milliseconds apart. Authorization is monotone under a single status
stamp: a duplicate status carrying a downgrade folds into the regime
itself, so a later numeric bearing that same status stamp can never
reverse a fail-closed decision. The rule fails closed across any source
identity, epoch, or clock change, beyond the skew budget, and for a
group acquired under a declared-unusable status. The guard is the same
shared ingress on every profile; only the numeric coherence budget is
profile-specific.
