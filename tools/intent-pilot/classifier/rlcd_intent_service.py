"""JSON-lines intent classifier for `intent-pilot`.

The service wraps the parallel constrained-decoding engine from the Hugging Face
repository harshatheg/Qwen-2.5-1B-RLCD. Put that repository on PYTHONPATH.

Protocol, one JSON object on each line:
  service -> pilot, one time:  {"ready": true, "model": "..."}
  pilot -> service:            {"message": "...", "targets": ["ALPHA", ...], "legend": "..."}
  service -> pilot:            {"target": "...", "on_arrival": "LAND"|"HOLD",
                                "target_prob": 0.97, "on_arrival_prob": 0.9, "model_ms": 140.0}
  service -> pilot, on fault:  {"error": "..."}

The prompt and the two field descriptions are the ones that were measured (probe v4):
the newest operator message and the waypoint legend, and nothing else. Do not add
vehicle state or message history. The measurements show that the model reads both
as instructions.
"""

import json
import os
import sys

# The engine prints progress text to stdout. The protocol owns stdout, so the
# engine's text goes to stderr.
PROTOCOL = os.fdopen(os.dup(sys.stdout.fileno()), "w", buffering=1)
sys.stdout = sys.stderr

from core import engine_mlx  # noqa: E402
from core.engine import get_engine, run_parallel_generation  # noqa: E402
from core.schema import StructuredSchema  # noqa: E402

TARGET_DESCRIPTION = (
    "The waypoint this operator message designates as the destination. "
    "HOME when the operator recalls the drone"
)
ARRIVAL_DESCRIPTION = (
    "What the operator wants at the destination: LAND there, or HOLD and wait there"
)


def send(payload):
    PROTOCOL.write(json.dumps(payload) + "\n")
    PROTOCOL.flush()


def build_schema(targets, tokenizer):
    schema = StructuredSchema(
        {
            "target": {"type": "enum", "choices": list(targets), "description": TARGET_DESCRIPTION},
            "on_arrival": {"type": "enum", "choices": ["LAND", "HOLD"], "description": ARRIVAL_DESCRIPTION},
        }
    )
    meta = schema.compile_parallel_metadata(tokenizer)
    collided = [name for (name, _), hit in zip(meta["field_items"], meta["has_collisions"]) if hit]
    if collided:
        # Two choices share a first token. The engine then guesses with a clamped
        # confidence, so refuse the names instead.
        raise ValueError(f"choices of {collided} share a first token; rename the waypoints")
    return schema


def main():
    _, tokenizer = get_engine()
    schemas = {}
    send({"ready": True, "model": engine_mlx.MODEL_ID})
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
            targets = tuple(request["targets"])
            if targets not in schemas:
                schemas[targets] = build_schema(targets, tokenizer)
            context = f"OPERATOR MESSAGE: {request['message']}\n{request['legend']}"
            result = run_parallel_generation(context, schemas[targets])
            fields = result["parsed_json"]
            send(
                {
                    "target": fields["target"]["value"],
                    "on_arrival": fields["on_arrival"]["value"],
                    "target_prob": fields["target"]["prob"],
                    "on_arrival_prob": fields["on_arrival"]["prob"],
                    "model_ms": result["elapsed_ms"],
                }
            )
        except Exception as fault:  # The pilot records the fault and keeps its destination.
            send({"error": f"{type(fault).__name__}: {fault}"})
    return 0


if __name__ == "__main__":
    sys.exit(main())
