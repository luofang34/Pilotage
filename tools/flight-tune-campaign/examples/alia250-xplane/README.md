# Alia 250 scenario matrix

This directory holds the generated scenario corpus for the Alia 250 campaign
and the generator that writes it.

## What is here

- `generate_matrix.py` — the generator. It is the authority for every byte
  under `conditions/` and `scenarios/`.
- `conditions/` — one condition artifact for each declared cell.
- `scenarios/` — one trial scenario for each declared cell.
- `manifest.json` — the corpus index, with a digest for each artifact.

## How to change the matrix

Change `generate_matrix.py`, run it, and commit what it writes:

```
python3 tools/flight-tune-campaign/examples/alia250-xplane/generate_matrix.py
```

Do not edit an artifact. `scripts/check-scenario-matrix-corpus.sh` regenerates
the corpus and refuses any file the generator would have written differently,
including a schema version someone changed by hand.

The Rust declaration in `tools/flight-tune-campaign/src/scenario/alia250.rs`
states the same matrix independently. Both halves must agree: the generator
writes the artifacts and the declaration says what they have to be, so a
generator that drifted produces artifacts the declaration rejects.

## Why the artifacts are one line

Each artifact is canonical compact JSON with no trailing newline, because a
condition identity is the digest of the exact artifact bytes. A pretty-printed
artifact decodes to the same values and hashes to a different identity, so the
scenario that names it would apply one disturbance and record another.

## What the matrix declares

Each of the fifteen stimuli flies the calm condition in every partition, and
each of the eleven uncertainty factors flies on one representative of each
control family in every partition. The artifact count follows from those two
rules rather than from a number anyone recorded.

SIM / NOT FOR FLIGHT.
