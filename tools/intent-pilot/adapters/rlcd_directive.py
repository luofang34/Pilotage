"""Directives from the parallel constrained-decoding engine of harshatheg/Qwen-2.5-1B-RLCD.

Put that repository on PYTHONPATH. The engine fills enumeration fields only, so each
number is a set of digit fields. The adapter decodes in two passes: the directive kind,
and then the slots of that kind.

The request text is the newest operator message and the legend, and nothing else.
"""

import sys

from adapter_common import ALL_KINDS, protect_stdout, serve, unable

PORT = protect_stdout()

from core import engine_mlx  # noqa: E402
from core.engine import get_engine, run_parallel_generation  # noqa: E402
from core.schema import StructuredSchema  # noqa: E402

DIGITS = [str(digit) for digit in range(10)]
KIND_CHOICES = [kind.upper() for kind in ALL_KINDS]
KIND_HELP = (
    "The flight instruction in the operator message. One of: TAKEOFF (leave the ground), "
    "DIRECT_TO (fly to a named fix), HEADING (turn to a compass heading in degrees), "
    "ALTITUDE (climb or descend to a height), SPEED (change speed), HOLD (stop and wait at a fix "
    "or at the present position), JOIN_PROCEDURE (fly a named approach procedure), "
    "LAND (land at the present position), GO_AROUND (abort a landing), "
    "RETURN_TO_BASE (fly back to the launch point and land), "
    "UNABLE (the message is not a flight instruction)"
)


def enum(choices, description):
    return {"type": "enum", "choices": list(choices), "description": description}


def compiled(fields, tokenizer):
    schema = StructuredSchema(fields)
    meta = schema.compile_parallel_metadata(tokenizer)
    collided = [name for (name, _), hit in zip(meta["field_items"], meta["has_collisions"]) if hit]
    if collided:
        # Two choices share a first token. The engine then guesses with a clamped
        # confidence, so refuse the names instead.
        raise ValueError(f"choices of {collided} share a first token")
    return schema


def slot_fields(kind, envelope):
    """The slots of one directive kind. A small schema for each kind reads better than
    one schema with every slot: in one large schema the fix slot lost its accuracy."""
    if kind == "direct_to":
        return {
            "fix": enum(envelope["fixes"], "The waypoint this operator message designates as the destination. "
                                           "HOME when the operator recalls the drone"),
            "on_arrival": enum(["LAND", "HOLD"], "What the operator wants at the destination: LAND there, or HOLD and wait there"),
        }
    if kind == "heading":
        return {
            "turn": enum(["LEFT", "RIGHT", "SHORTEST"],
                         "The side of the turn that the message names. SHORTEST when it names no side"),
            "heading_hundreds": enum(DIGITS[:4], "The first digit of the three-digit heading in the message"),
            "heading_tens": enum(DIGITS, "The second digit of the three-digit heading in the message"),
            "heading_ones": enum(DIGITS, "The third digit of the three-digit heading in the message"),
        }
    if kind == "altitude":
        return {
            "height_tens": enum(DIGITS[:4], "The tens digit of the height in metres in the message. 0 when the height is below 10"),
            "height_ones": enum(DIGITS, "The ones digit of the height in metres in the message"),
        }
    if kind == "speed":
        return {
            "speed_ones": enum(DIGITS[:4], "The whole metres per second of the speed in the message"),
            "speed_tenths": enum(["0", "5"], "The tenths of the speed in the message. 0 for a whole number"),
        }
    if kind == "hold":
        return {"hold_point": enum(["PRESENT_POSITION"] + list(envelope["fixes"]),
                                   "Where to hold: a named fix, or PRESENT_POSITION when the message names no fix")}
    if kind == "join_procedure" and envelope["procedures"]:
        return {"procedure": enum(envelope["procedures"], "The named approach procedure in the message")}
    return {}


def main():
    _, tokenizer = get_engine()
    schemas = {}

    def decode(name, fields, context):
        if name not in schemas:
            schemas[name] = compiled(fields, tokenizer)
        return run_parallel_generation(context, schemas[name])["parsed_json"]

    def answer(request):
        envelope = request["envelope"]
        names = (tuple(envelope["fixes"]), tuple(envelope["procedures"]))
        context = f"OPERATOR MESSAGE: {request['message']}\n{request['legend']}"
        first = decode(("kind",), {"kind": enum(KIND_CHOICES, KIND_HELP)}, context)
        kind = first["kind"]["value"].lower()
        probabilities = {"kind": round(first["kind"]["prob"], 4)}
        fields = slot_fields(kind, envelope)
        got = decode((kind, names), fields, context) if fields else {}
        value = lambda name: got[name]["value"]
        prob = lambda *names_: round(min(got[name]["prob"] for name in names_), 4)
        if kind == "direct_to":
            probabilities.update(fix=prob("fix"), on_arrival=prob("on_arrival"))
            return {"kind": kind, "fix": value("fix"), "on_arrival": value("on_arrival").lower()}, probabilities
        if kind == "heading":
            digits = ("heading_hundreds", "heading_tens", "heading_ones")
            degrees = int("".join(value(name) for name in digits))
            probabilities.update(degrees=prob(*digits), turn=prob("turn"))
            if degrees > 360:
                return unable(f"the digits give heading {degrees}"), probabilities
            return {"kind": kind, "degrees": degrees % 360, "turn": value("turn").lower()}, probabilities
        if kind == "altitude":
            digits = ("height_tens", "height_ones")
            probabilities.update(height_m=prob(*digits))
            return {"kind": kind, "height_m": float(int(value(digits[0]) + value(digits[1])))}, probabilities
        if kind == "speed":
            digits = ("speed_ones", "speed_tenths")
            probabilities.update(speed_mps=prob(*digits))
            return {"kind": kind, "speed_mps": float(f"{value(digits[0])}.{value(digits[1])}")}, probabilities
        if kind == "hold":
            probabilities.update(point=prob("hold_point"))
            point = value("hold_point")
            where = {"at": "present_position"} if point == "PRESENT_POSITION" else {"at": "fix", "fix": point}
            return {"kind": kind, "point": where}, probabilities
        if kind == "join_procedure":
            if not fields:
                return unable("the envelope has no procedure"), probabilities
            probabilities.update(procedure=prob("procedure"))
            return {"kind": kind, "procedure": value("procedure")}, probabilities
        if kind == "unable":
            return unable("the model read no flight instruction"), probabilities
        return {"kind": kind}, probabilities

    serve(PORT, {"adapter": "rlcd-parallel-two-stage/3", "model": engine_mlx.MODEL_ID + " (parallel constrained decoding)",
                 "kinds": ALL_KINDS, "frames": {"max_frames": 0, "projections": []}}, answer)
    return 0


if __name__ == "__main__":
    sys.exit(main())
