#!/usr/bin/env python3
"""Generate the Alia 250 scenario matrix from its canonical inputs.

The corpus this writes is the authority for what a campaign executes, and it
is generated rather than maintained: a hand-edited artifact is one nobody
re-derived, and the coverage rules count a factor only when the artifact
carries its exact executable value.

Every file is canonical compact JSON with no trailing newline, because the
condition identity is the SHA-256 of the exact artifact bytes. A pretty-printed
artifact would decode to the same values and hash to a different identity.

SIM / NOT FOR FLIGHT.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import sys
from pathlib import Path

SCENARIO_SCHEMA_VERSION = 3
CONDITION_SCHEMA_VERSION = 4
BASIS_POINTS_NOMINAL = 10000

DEGREE = 0.017453292519943295

# The three isolated partitions. A candidate fitted to a training disturbance
# meets a different one on the run that decides what ships, so each partition
# carries its own seed stream and its own artifact identities.
PARTITIONS = ("training", "promotion", "final")

# The stimulus each cell commands. The envelope states what the normalized
# range is worth in physical units; the normalized value states how much of it
# this trial asks for.
STIMULI = (
    # Direct attitude steps. The envelope spans plus and minus twenty degrees,
    # so a quarter of it is the five degree step and all of it is the twenty.
    ("roll-step-5deg", "direct_attitude_thrust", "roll", "alia.direct.roll", 20.0 * DEGREE, 0.25),
    ("roll-step-10deg", "direct_attitude_thrust", "roll", "alia.direct.roll", 20.0 * DEGREE, 0.5),
    ("roll-step-20deg", "direct_attitude_thrust", "roll", "alia.direct.roll", 20.0 * DEGREE, 1.0),
    ("pitch-step-5deg", "direct_attitude_thrust", "pitch", "alia.direct.pitch", 20.0 * DEGREE, 0.25),
    ("pitch-step-10deg", "direct_attitude_thrust", "pitch", "alia.direct.pitch", 20.0 * DEGREE, 0.5),
    ("pitch-step-20deg", "direct_attitude_thrust", "pitch", "alia.direct.pitch", 20.0 * DEGREE, 1.0),
    ("yaw-step-10deg", "direct_attitude_thrust", "yaw", "alia.direct.yaw", 20.0 * DEGREE, 0.5),
    # Return to trim. The reversal leaves the step and comes back, which is
    # what the opposite return peak and the final body-rate measure.
    ("roll-return-zero", "direct_attitude_thrust", "roll", "alia.direct.roll", 20.0 * DEGREE, 0.5),
    ("pitch-return-zero", "direct_attitude_thrust", "pitch", "alia.direct.pitch", 20.0 * DEGREE, 0.5),
    # Direct collective force, normalized against the identified hover force.
    ("collective-step-up", "direct_attitude_thrust", "vertical", "alia.direct.collective", 0.3, 0.5),
    ("collective-step-down", "direct_attitude_thrust", "vertical", "alia.direct.collective", 0.3, -0.5),
    # Operator command families.
    ("operator-roll-velocity", "operator_velocity", "roll", "alia.operator.horizontal", 5.0, 0.85),
    ("operator-pitch-velocity", "operator_velocity", "pitch", "alia.operator.horizontal", 5.0, 0.85),
    ("operator-vertical-velocity", "operator_velocity", "vertical", "alia.operator.vertical", 3.0, 0.85),
    ("operator-yaw-rate", "operator_velocity", "yaw", "alia.operator.yaw", 1.5, 0.85),
)

# One representative of each control family carries every uncertainty factor,
# so no factor is covered on one family only.
FAMILY_REPRESENTATIVES = ("roll-step-10deg", "operator-roll-velocity")

# The calm condition every stimulus flies, and the uncertainty factors the
# representatives fly. Each entry names the exact executable value the
# artifact has to carry.
CONDITIONS = (
    ("calm", {}),
    ("crosswind", {"steady_speed_mps": 5.0, "steady_direction_deg": 270.0}),
    ("headwind", {"steady_speed_mps": 5.0, "steady_direction_deg": 0.0}),
    ("gust-release", {"gust_speed_mps": 5.0, "gust_hold_ns": 1000000000}),
    ("authority-high", {"authority_scale_basis_points": 12000}),
    ("authority-low", {"authority_scale_basis_points": 8000}),
    ("hover-trim-high", {"hover_scale_basis_points": 11000}),
    ("hover-trim-low", {"hover_scale_basis_points": 9000}),
    ("sensor-noise", {"sensor_noise": True}),
    ("timing-jitter", {"jitter_maximum_delay_ns": 4000000, "jitter_interval_ns": 250000000}),
    ("added-delay", {"estimate_delay_ns": 30000000}),
    ("command-loss", {"loss_fraction_basis_points": 100, "loss_interval_samples": 100}),
)

CALM = CONDITIONS[0][0]
UNCERTAINTY = tuple(name for name, _ in CONDITIONS[1:])

# The sensor lanes a noise request declares, with the peak amplitude and the
# update interval in samples that the coverage rule compares against.
SENSOR_LANES = (
    ("accelerometer", "x", "peak_amplitude_mps2", 0.15, 10),
    ("accelerometer", "y", "peak_amplitude_mps2", 0.15, 10),
    ("accelerometer", "z", "peak_amplitude_mps2", 0.2, 10),
    ("gyroscope", "x", "peak_amplitude_rad_s", 0.01, 5),
    ("gyroscope", "y", "peak_amplitude_rad_s", 0.01, 5),
    ("gyroscope", "z", "peak_amplitude_rad_s", 0.01, 5),
)

PHASE_START_NS = 2000000000
PHASE_HOLD_NS = 4000000000
PHASE_RELEASE_NS = 7000000000
COMPLETION_NS = 10000000000
PHASE_CEILING_NS = 12000000000

# The domain that separates one partition's seed stream from another.
PARTITION_SEED_DOMAIN = {
    "training": "pilotage.alia250.matrix.training",
    "promotion": "pilotage.alia250.matrix.promotion",
    "final": "pilotage.alia250.matrix.final",
}


def canonical(document: object) -> bytes:
    """Encode one artifact exactly as the trial contract encodes it."""
    return json.dumps(document, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def digest(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def run_seed(partition: str, stimulus: str, condition: str) -> int:
    """The deterministic seed one cell runs under.

    The partition domain separates the three streams, so no training seed can
    reappear on the run that decides what ships.
    """
    material = f"{PARTITION_SEED_DOMAIN[partition]}\0{stimulus}\0{condition}".encode("utf-8")
    return int.from_bytes(hashlib.sha256(material).digest()[:8], "little")


def wind(values: dict) -> dict:
    gusts = []
    if "gust_speed_mps" in values:
        gusts.append(
            {
                "start_ns": PHASE_HOLD_NS,
                "rise_ns": 250000000,
                "hold_ns": values["gust_hold_ns"],
                "fall_ns": 250000000,
                "speed_mps": values["gust_speed_mps"],
                "direction_deg": 270.0,
            }
        )
    return {
        "steady": {
            "speed_mps": values.get("steady_speed_mps", 0.0),
            "direction_deg": values.get("steady_direction_deg", 0.0),
        },
        "gusts": gusts,
        "turbulence": {"kind": "none"},
    }


def timing(values: dict) -> dict:
    jitter = {"kind": "none"}
    if "jitter_maximum_delay_ns" in values:
        jitter = {
            "kind": "sample_and_hold",
            "maximum_delay_ns": values["jitter_maximum_delay_ns"],
            "interval_ns": values["jitter_interval_ns"],
        }
    return {
        "estimate_delay_ns": values.get("estimate_delay_ns", 0),
        "update_jitter": jitter,
    }


def sensor(values: dict) -> dict:
    if not values.get("sensor_noise"):
        return {"kind": "none"}
    lanes = []
    for name, axis, amplitude_field, amplitude, interval in SENSOR_LANES:
        lanes.append(
            {
                "sensor": name,
                "axis": axis,
                amplitude_field: amplitude,
                "update_interval_samples": interval,
            }
        )
    return {"kind": "bounded_noise", "lanes": lanes}


def actuator(values: dict) -> dict:
    loss = {"kind": "none"}
    if "loss_fraction_basis_points" in values:
        loss = {
            "kind": "seeded_zero_order_hold",
            "fraction_basis_points": values["loss_fraction_basis_points"],
            "decision_interval_samples": values["loss_interval_samples"],
        }
    return {
        "authority_scale_basis_points": values.get(
            "authority_scale_basis_points", BASIS_POINTS_NOMINAL
        ),
        "command_loss": loss,
    }


def controller_initialization(values: dict) -> dict:
    return {
        "hover_thrust_force": {
            "kind": "scale_baseline",
            "scale_basis_points": values.get("hover_scale_basis_points", BASIS_POINTS_NOMINAL),
        }
    }


def plant(values: dict) -> dict:
    """The simulated aircraft one cell flies.

    This matrix varies the disturbance and the controller, never the airframe,
    so every cell declares the baseline mass and center of gravity and accepts
    the hover ratio the simulator reports. The block is still written because
    the executor requires it, and an artifact that omitted it would not load.
    """
    return {
        "payload_mass_delta_kg": values.get("payload_mass_delta_kg", 0.0),
        "longitudinal_cg_offset_m": values.get("longitudinal_cg_offset_m", 0.0),
        "lateral_cg_offset_m": values.get("lateral_cg_offset_m", 0.0),
        "hover_thrust_expectation": {"kind": "measured_weight_ratio"},
    }


def condition_set(partition: str, name: str, stimulus: str, values: dict, seed: int) -> dict:
    # The field order here is the canonical encoding order, so it must match
    # the declaration order of the condition contract exactly.
    return {
        "schema_version": CONDITION_SCHEMA_VERSION,
        # One artifact carries one identity. Two cells that applied the same
        # named condition to different stimuli hold different seeds, so an
        # identity that named only the condition would be two artifacts under
        # one name.
        "id": f"alia250.{partition}.{name}.{stimulus}",
        "revision": 1,
        "seed": seed,
        "wind": wind(values),
        "timing": timing(values),
        "sensor": sensor(values),
        "actuator": actuator(values),
        "controller_initialization": controller_initialization(values),
        "plant": plant(values),
    }


def envelope(family: str, channel: str, envelope_id: str, endpoint: float) -> dict:
    if family == "operator_velocity":
        unit = "radians_per_second" if channel == "yaw" else "meters_per_second"
        reference = "zero"
    elif channel == "vertical":
        unit = "normalized_collective_force"
        reference = "identified_hover_trim"
    else:
        unit = "radians"
        reference = "effective_setpoint_at_entry"
    return {
        "id": envelope_id,
        "revision": 1,
        "unit": unit,
        "reference": reference,
        "negative_endpoint": -endpoint,
        "neutral": 0.0,
        "positive_endpoint": endpoint,
    }


def waveform(stimulus: str, value: float) -> dict:
    if stimulus.endswith("-return-zero"):
        return {
            "kind": "reversal",
            "first": value,
            "second": 0.0,
            "dwell_ns": PHASE_RELEASE_NS - PHASE_HOLD_NS,
        }
    return {"kind": "step", "value": value}


def scenario(
    partition: str,
    stimulus: tuple,
    condition_name: str,
    condition_id: str,
    condition_digest: str,
) -> dict:
    name, family, channel, envelope_id, endpoint, value = stimulus
    capability = (
        "operator_velocity_control"
        if family == "operator_velocity"
        else "direct_attitude_thrust_control"
    )
    mapping = "candidate_bound_curve" if family == "operator_velocity" else "affine_exact"
    return {
        "schema_version": SCENARIO_SCHEMA_VERSION,
        "id": f"alia250.{partition}.{name}.{condition_name}",
        "revision": 1,
        "phases": [
            {
                "id": "conditions",
                "max_sim_time_ns": PHASE_CEILING_NS,
                "required_capabilities": ["simulator_time", "condition_control"],
                "entry_conditions": [{"kind": "always"}],
                "action": {
                    "kind": "apply_conditions",
                    "condition_set": {
                        "id": condition_id,
                        "revision": "1",
                        "digest": condition_digest,
                    },
                },
                "exit_conditions": [{"kind": "always"}],
                "abort_conditions": [],
            },
            {
                "id": "settle",
                "max_sim_time_ns": PHASE_CEILING_NS,
                "required_capabilities": ["simulator_time", "contact_state"],
                "entry_conditions": [{"kind": "always"}],
                "action": {"kind": "settle"},
                "exit_conditions": [
                    {
                        "kind": "simulator_time",
                        "comparison": "greater_or_equal",
                        "value_ns": PHASE_START_NS,
                    }
                ],
                "abort_conditions": [{"kind": "crashed", "expected": True}],
            },
            {
                "id": "stimulate",
                "max_sim_time_ns": PHASE_CEILING_NS,
                "required_capabilities": ["simulator_time", "contact_state", capability],
                "entry_conditions": [{"kind": "always"}],
                "action": {
                    "kind": "stimulus",
                    "family": family,
                    "channel": channel,
                    "mapping": mapping,
                    "envelope": envelope(family, channel, envelope_id, endpoint),
                    "waveform": waveform(name, value),
                },
                "exit_conditions": [
                    {
                        "kind": "simulator_time",
                        "comparison": "greater_or_equal",
                        "value_ns": PHASE_RELEASE_NS,
                    }
                ],
                "abort_conditions": [{"kind": "crashed", "expected": True}],
            },
            {
                "id": "observe",
                "max_sim_time_ns": PHASE_CEILING_NS,
                "required_capabilities": ["simulator_time", "contact_state"],
                "entry_conditions": [{"kind": "always"}],
                "action": {"kind": "observe"},
                "exit_conditions": [
                    {
                        "kind": "simulator_time",
                        "comparison": "greater_or_equal",
                        "value_ns": COMPLETION_NS,
                    }
                ],
                "abort_conditions": [{"kind": "crashed", "expected": True}],
            },
        ],
    }


def cells() -> list[tuple[str, tuple, str]]:
    """Every cell the matrix declares, in one stable order.

    Each stimulus flies calm in every partition, and each uncertainty factor
    flies on one representative of each control family in every partition. The
    count follows from those two rules, so a missing or an extra file is a
    difference from the declaration rather than from a number someone wrote
    down.
    """
    declared = []
    for partition in PARTITIONS:
        for stimulus in STIMULI:
            declared.append((partition, stimulus, CALM))
        for condition in UNCERTAINTY:
            for name in FAMILY_REPRESENTATIVES:
                stimulus = next(entry for entry in STIMULI if entry[0] == name)
                declared.append((partition, stimulus, condition))
    return declared


def generate() -> dict[str, bytes]:
    """The complete corpus, as a path-to-bytes map."""
    files: dict[str, bytes] = {}
    conditions = dict(CONDITIONS)
    for partition, stimulus, condition_name in cells():
        name = stimulus[0]
        seed = run_seed(partition, name, condition_name)
        condition = condition_set(
            partition, condition_name, name, conditions[condition_name], seed
        )
        condition_bytes = canonical(condition)
        condition_path = f"conditions/{partition}.{condition_name}.{name}.json"
        files[condition_path] = condition_bytes
        document = scenario(
            partition,
            stimulus,
            condition_name,
            condition["id"],
            digest(condition_bytes),
        )
        files[f"scenarios/{partition}.{name}.{condition_name}.json"] = canonical(document)
    return files


def manifest(files: dict[str, bytes]) -> bytes:
    """The corpus index a checker reads before it opens one artifact."""
    entries = [
        {"path": path, "digest": digest(payload)} for path, payload in sorted(files.items())
    ]
    return canonical(
        {
            "schema_version": 1,
            "matrix_id": "alia250-xplane",
            "partitions": list(PARTITIONS),
            "stimuli": [entry[0] for entry in STIMULI],
            "conditions": [name for name, _ in CONDITIONS],
            "family_representatives": list(FAMILY_REPRESENTATIVES),
            "cell_count": len(cells()),
            "document_count": len(files),
            "files": entries,
        }
    )


def write(root: Path, files: dict[str, bytes]) -> None:
    for directory in ("conditions", "scenarios"):
        target = root / directory
        if target.exists():
            shutil.rmtree(target)
        target.mkdir(parents=True)
    for path, payload in files.items():
        (root / path).write_bytes(payload)
    (root / "manifest.json").write_bytes(manifest(files))


def check(root: Path, files: dict[str, bytes]) -> int:
    """Compare the checked-in corpus with what this generator produces."""
    failures = []
    for path, payload in sorted(files.items()):
        target = root / path
        if not target.is_file():
            failures.append(f"missing {path}")
        elif target.read_bytes() != payload:
            failures.append(f"changed {path}")
    for directory in ("conditions", "scenarios"):
        for target in sorted((root / directory).glob("*.json")):
            relative = f"{directory}/{target.name}"
            if relative not in files:
                failures.append(f"orphan {relative}")
    expected_manifest = manifest(files)
    manifest_path = root / "manifest.json"
    if not manifest_path.is_file() or manifest_path.read_bytes() != expected_manifest:
        failures.append("changed manifest.json")
    for failure in failures:
        print(f"generate_matrix: {failure}", file=sys.stderr)
    if failures:
        print(f"generate_matrix: {len(failures)} corpus differences", file=sys.stderr)
        return 1
    print(f"generate_matrix: OK, {len(files)} artifacts and one manifest")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="compare the checked-in corpus with the generator output",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parent,
        help="the directory the corpus is written to or checked against",
    )
    arguments = parser.parse_args()
    files = generate()
    if arguments.check:
        return check(arguments.out, files)
    write(arguments.out, files)
    print(f"generate_matrix: wrote {len(files)} artifacts and one manifest")
    return 0


if __name__ == "__main__":
    sys.exit(main())
