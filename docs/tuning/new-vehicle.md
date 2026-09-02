# Add a vehicle to a tuning campaign

This document is a checklist. It states the surfaces you touch to add a new
vehicle to a flight-tune campaign, in order, with the exact path for each
surface and the guard or test that proves you did it correctly. When a guard
exists, this document names it instead of restating what it checks.

Two vehicles exist today. The Alia 250 carries all seven surfaces below and
can run a real campaign. The x500 carries only the first two: it has a bar
and a bench model, so the CI-fast certification test flies it, but it has no
scenario matrix and no corpus, so it cannot fly a real campaign yet. The
worked example at the end of this document uses the x500 to show what a
partly onboarded vehicle looks like.

SIM / NOT FOR FLIGHT.

## The seven surfaces

### 1. The objective limit table and the policy constructors

Path: `tools/flight-tune-campaign/src/vehicle_policy.rs`.

Add a `<VEHICLE>_OBJECTIVE_LIMITS` table: one row per objective, each row a
`(name, promotion regression limit, final absolute maximum)` triple. Add the
four constructors that read the table: `<vehicle>_promotion_policy`,
`<vehicle>_qualification_policy`, `<vehicle>_response_targets`, and
`<vehicle>_required_policy`. Every objective name must be one
`pilotage_flight_quality::is_producible` (in
`crates/pilotage-flight-quality/src/vocabulary.rs`) already knows. A bar that
names a metric nothing measures fails the whole campaign on the name, after
every run has flown.

Guard: `cargo test -p flight-tune-campaign` runs `vehicle_policy/tests.rs`.
Two tests there check a new vehicle, but only if you add it to their arrays:
`every_vehicle_states_its_bar_over_metrics_the_scoring_layer_produces` (every
named objective is producible, and the promotion and qualification halves
name the same objectives), and `each_absolute_ceiling_sits_above_its_regression_limit`
(the final ceiling sits above the promotion limit for every objective).
Neither test discovers a new vehicle on its own: add the vehicle to each
array, or the guard silently skips it. `alia_policy_limits_are_finite_and_nonnegative`
checks the same finite-and-nonnegative property but is written against the
Alia policy only; copy it for the new vehicle rather than widening it.

### 2. The bench warm-start plant

Path: `tools/flight-tune-campaign/src/bench.rs`, the `BenchVehicle` type.

Add a `BenchVehicle::<vehicle>()` constructor with two numbers: the velocity
time constant in seconds and the full-scale velocity in metres per second.
This is the reduced first-order plant the fast, CI-runnable certification
test flies the shaped command law against; it is not a simulator, and a
result from it is not a qualified calibration for the aircraft.

Guard: `cargo test -p flight-tune-campaign` runs `bench/tests.rs`, which also
needs the new vehicle added to its arrays:
`each_vehicle_states_a_finite_bound_inside_its_declared_budget` (the search
fits its declared run and duration budget) and `probe_warm_start_objectives`
(the calibration probe in the next section). Add a new `#[ignore]`
certification test named `a_campaign_runs_end_to_end_for_the_<vehicle>`,
mirroring the existing Alia 250 and x500 ones, so the new vehicle's full
campaign is exercised on its own CI step.

### 3. The Rust matrix declaration

Path: `tools/flight-tune-campaign/src/scenario/<vehicle>.rs`.

Declare a `const <VEHICLE>_MATRIX: ScenarioMatrix` with every `MatrixStimulus`,
every `MatrixCondition`, and the family representatives. This is one of the
two independent statements of the matrix; see "The double-statement rule"
below. Add `mod <vehicle>;` and `pub use <vehicle>::<VEHICLE>_MATRIX;` to
`tools/flight-tune-campaign/src/scenario.rs`.

Guard: none by itself. It is checked jointly with surface 5 below.

### 4. The Python matrix generator and its vehicle table

Path: `tools/flight-tune-campaign/examples/<vehicle>-<sim>/generate_matrix.py`
and a vehicle table file beside it (`<vehicle>.vehicle.json`).

One generator script now serves every vehicle. Copy an existing example
directory's `generate_matrix.py` unchanged, and write a new
`<vehicle>.vehicle.json` beside it: the vehicle identity, the matrix
identity, the family representatives, and the stimuli, each with its
envelope identity and its physical endpoint. The copy needs no edit: with no
`--vehicle-table` argument, the generator finds the one `*.vehicle.json` file
beside itself. The uncertainty factors, the sensor lanes, and the phase
timings stay in the script: they are the campaign's method, not this
vehicle's physics, and a vehicle that needs a different one is a method
change, not a table edit.

Guard: `scripts/check-scenario-matrix-corpus.sh` (the byte-identity half of
"The double-statement rule") proves this for the Alia directory. It does not
yet generalize: the script names `examples/alia250-xplane/generate_matrix.py`
by path, and its self-test (`scripts/test-check-scenario-matrix-corpus.sh`)
does the same. A second vehicle's corpus is unguarded until both scripts
learn its directory. Per this repository's shell-script rule (`AGENTS.md`),
that is a decision, so it moves into a Rust guard under `tools/xtask` rather
than growing the shell scripts; treat it as a prerequisite step for a second
vehicle, not something this checklist can promise is already generic. A
malformed vehicle table fails at load time, before anything is generated,
with a `ValueError` naming the missing field, whether the field is missing at
the top level or inside one stimulus entry.

### 5. The checked-in corpus

Path: `tools/flight-tune-campaign/examples/<vehicle>-<sim>/conditions/`,
`.../scenarios/`, and `.../manifest.json`.

Run the generator and commit exactly what it writes. Never hand-edit an
artifact: a condition's identity is the SHA-256 of its exact canonical
bytes, so a hand-edited byte changes the identity without changing what a
checker reads back as "this artifact".

Guard: the same `scripts/check-scenario-matrix-corpus.sh` proves the corpus
is the generator's output. `cargo test -p flight-tune-campaign` separately
proves the corpus satisfies the Rust declaration from surface 3, through
`LoadedMatrix::load_blocking` and its coverage check
(`tools/flight-tune-campaign/src/scenario/matrix.rs`).

### 6. The stimulus envelope identities and physical endpoints

These live inside surfaces 3 and 4, not in a file of their own: the same
envelope identity (for example `alia.direct.roll`) and the same physical
endpoint must appear in the Rust `MatrixStimulus` list and in the vehicle
table's `stimuli` entries.

Guard: partial. `LoadedMatrix::load_blocking`'s `verify_cell`
(`tools/flight-tune-campaign/src/scenario/matrix.rs`) checks that a corpus
scenario names the declared control family, channel, and envelope identity
string. It does not decode and compare the envelope's numeric endpoint or the
waveform's normalized value against the Rust declaration's
`positive_endpoint` and `normalized_value`. A Rust declaration whose endpoint
disagrees with the vehicle table's, while both still produce a self-consistent
corpus, is not caught by an existing guard: keep the two numbers equal by
reading both when you change either.

### 7. The Aviate side

The application that flies a candidate in a real simulator lives in the
Aviate repository, not here. This document names the shape and stops:

- One application crate per vehicle and simulator pair, named
  `aviate-apps/sitl-<simulator>-<vehicle>` (for example
  `sitl-xplane-alia250`). Its `AviateApp.toml` states the airframe and the
  simulator board.
- One preset per vehicle (and, where the physics need it, per simulator),
  under `presets/`.
- One wire identity: the transport endpoint the application declares (for
  example a fixed MAVLink UDP port), and the Pilotage side's own selection of
  it, which for a live campaign is a per-vehicle backend under
  `tools/xtask/src/backend/` naming the application binary and the airframe
  (for example `aviate_xplane.rs` for the Alia 250 and X-Plane pair).

Read the Aviate repository for how these three pieces work. This document
does not describe Aviate internals.

## The double-statement rule

The matrix is stated twice on purpose: the Python generator (surface 4)
writes the corpus, and the Rust declaration (surface 3) states what a
correct corpus has to contain. Neither statement reads the other. Two
separate checks meet at the corpus in between them, and each proves a
different half:

- `scripts/check-scenario-matrix-corpus.sh` regenerates the corpus from the
  Python generator and refuses any byte the checked-in corpus states
  differently. This proves generator output equals checked-in corpus.
- `cargo test -p flight-tune-campaign` loads the checked-in corpus against
  the Rust `ScenarioMatrix` declaration (`LoadedMatrix::load_blocking` and
  its coverage check). This proves checked-in corpus satisfies the Rust
  declaration.

A generator that drifted from the Rust declaration produces a corpus the
first check accepts and the second refuses. A Rust declaration that drifted
from the generator does the reverse. A generator and a declaration that
drifted together, to the same wrong matrix, pass both checks: the rule
catches an accidental disagreement between the two statements, not a mistake
they share. Surface 6 above names one further limit: even when both checks
pass, an endpoint value is not one of the things either check compares.

## Corpus regeneration workflow

After changing a vehicle table or a Rust matrix declaration:

1. Edit the vehicle table (`<vehicle>.vehicle.json`) for a stimulus, an
   envelope, or an endpoint change; edit `scenario/<vehicle>.rs` to match.
   A mismatched id, family, channel, or envelope identity is caught by
   step 3. A mismatched endpoint or normalized value is not (surface 6):
   keep those two numbers equal by reading both files, not by relying on a
   guard to catch a disagreement between them.
2. Regenerate: `python3 tools/flight-tune-campaign/examples/<vehicle>-<sim>/generate_matrix.py`.
   Commit what it writes.
3. Prove both halves of the double-statement rule:
   `cargo run -p xtask -- guards` (discovers and runs every
   `scripts/check-*.sh` / `scripts/test-check-*.sh` pair, including the
   corpus byte-identity guard), then `cargo test -p flight-tune-campaign`
   (the Rust-declaration-against-corpus check, and every other unit test in
   the crate).
4. Before a release, also run the full certification suite:
   `cargo test -p flight-tune-campaign -- --ignored --test-threads 2`. CI
   runs this as its own "Campaign certification" step, separate from the
   main test suite, because it flies a complete search, promotion, and
   final-qualification chain for every vehicle.

## The bench-probe recipe for deriving objective limits

A vehicle's objective limits (surface 1) and operator-authority floor are not
guessed. They come from running the shipped command law on the bench plant
(surface 2) and reading what it actually measures.

1. Add the vehicle to `probe_warm_start_objectives`'s array in
   `tools/flight-tune-campaign/src/bench/tests.rs` (see surface 2).
2. Run the probe and read the shipped law's measured values:
   `cargo test -p flight-tune-campaign probe_warm_start_objectives -- --ignored --nocapture`.
   This prints every objective the shipped warm start measures on this
   vehicle's bench trial, including `authority.resolved_target`
   (`flight_tune::TARGET_AUTHORITY_OBJECTIVE`): the physical speed the
   shipped law resolved for the trial's held operator input.
3. Set each promotion regression limit to admit the printed value with
   margin for the search's neighborhood, and each final absolute maximum
   above that, loose enough to admit a legal candidate and tight enough to
   refuse a serious regression. Both existing vehicles scale most objectives
   by a factor of ten from the regression limit to the final maximum; treat
   that as an observed starting point, not a rule, and confirm the ordering
   with `each_absolute_ceiling_sits_above_its_regression_limit`
   (`vehicle_policy/tests.rs`).
4. Set the operator-authority floor
   (`MINIMUM_OPERATOR_AUTHORITY` in `vehicle_policy.rs`) as a fraction of
   `bench_physical_target` (`tools/flight-tune-campaign/src/bench/qualifying/trial.rs`).
   The floor bounds the physical target the candidate actually resolved on
   the run, not the one the scenario requested, so a candidate cannot buy a
   better normalized response by quietly asking the vehicle for less. Start
   from the existing fraction; only widen it with evidence that a legal
   candidate the current floor accepts is one that gave away authority.
5. Do not commit the printed numbers into this document: read them from a
   fresh probe run when you need them, so nothing here goes stale when the
   trial or the models change.

## Worked example: the x500's partial onboarding

The x500 shows what a vehicle looks like partway through this checklist. It
has surface 1 (`X500_OBJECTIVE_LIMITS` and its four policy constructors) and
surface 2 (`BenchVehicle::x500()`, enrolled in the bench certification and
probe arrays), so `cargo test -p flight-tune-campaign` and the ignored
`a_campaign_runs_end_to_end_for_the_x500` certification test both fly it
against the bench plant. `adapters/aviate/profiles/x500-shaped-*.json` also
carries a shaped control-feel profile for it, and the Aviate repository has
a general-purpose simulator launch entry for it
(`aviate-apps/sitl-gazebo-x500`, used by `cargo xtask sim` for a live flight
session). None of that is surfaces 3 through 6: the x500 has no
`scenario/x500.rs` matrix declaration, no `examples/x500-<sim>/` generator or
vehicle table, and no checked-in corpus, so it cannot fly a real tuning
campaign yet. Completing this document's checklist for the x500 is what
would close that gap.
