"""Shared parts of the model adapters: the JSON-lines port and directive assembly.

An adapter is a separate program. It writes its declaration as its first line. After
that it reads one request line and writes one reply line. A reply is
{"directive": {...}, "probabilities": {...}, "model_ms": 0.0}, or {"error": "..."}.
"""

import json
import os
import sys
import time

ALL_KINDS = ["takeoff", "direct_to", "heading", "altitude", "speed", "hold",
             "join_procedure", "land", "go_around", "return_to_base", "unable"]


def protect_stdout():
    """Returns the protocol stream. Every other write to stdout goes to stderr.

    A model runtime prints progress text to stdout, and the port owns stdout. A
    native library writes to file descriptor 1 and not to sys.stdout, so the
    descriptor is moved too.
    """
    port = os.fdopen(os.dup(sys.stdout.fileno()), "w", buffering=1)
    os.dup2(sys.stderr.fileno(), sys.stdout.fileno())
    sys.stdout = sys.stderr
    return port


def serve(port, declaration, answer):
    """Runs the port loop. `answer(request)` gives (directive, probabilities)."""
    port.write(json.dumps(dict(declaration, ready=True)) + "\n")
    port.flush()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        started = time.perf_counter()
        request_id = 0
        try:
            request = json.loads(line)
            request_id = int(request.get("id", 0))
            directive, probabilities = answer(request)
            reply = {"id": request_id, "directive": directive, "probabilities": probabilities,
                     "model_ms": round((time.perf_counter() - started) * 1000.0, 1)}
        except Exception as fault:  # The caller records the fault and keeps its directive.
            reply = {"id": request_id, "error": f"{type(fault).__name__}: {fault}"}
        port.write(json.dumps(reply) + "\n")
        port.flush()


def unable(reason):
    return {"kind": "unable", "reason": reason}


def clean_directive(raw, envelope):
    """Keeps the slots of the directive kind and drops every other member.

    A generative model adds members of its own. The port refuses unknown members, so
    the adapter owns this step. It does not change a slot value.
    """
    kind = str(raw.get("kind", "")).lower()
    if kind == "direct_to":
        return {"kind": kind, "fix": str(raw.get("fix", "")).upper(),
                "on_arrival": str(raw.get("on_arrival", "")).lower()}
    if kind == "heading":
        return {"kind": kind, "degrees": int(raw["degrees"]) % 360,
                "turn": str(raw.get("turn", "shortest")).lower()}
    if kind == "altitude":
        return {"kind": kind, "height_m": float(raw["height_m"])}
    if kind == "speed":
        return {"kind": kind, "speed_mps": float(raw["speed_mps"])}
    if kind == "hold":
        point = raw.get("point") or {}
        fix = point.get("fix") if isinstance(point, dict) else None
        if fix:
            return {"kind": kind, "point": {"at": "fix", "fix": str(fix).upper()}}
        return {"kind": kind, "point": {"at": "present_position"}}
    if kind == "join_procedure":
        return {"kind": kind, "procedure": str(raw.get("procedure", "")).upper()}
    if kind in ("takeoff", "land", "go_around", "return_to_base"):
        return {"kind": kind}
    if kind == "unable":
        return unable(str(raw.get("reason", "the model gave no reason")))
    # An unknown kind is a fault of the model and not an "unable" answer: an
    # "unable" scores as correct on a message that is not an instruction.
    raise ValueError(f"the model gave no usable kind: {raw.get('kind')!r}")


JSON_FORMS = """Reply with ONE JSON object and nothing else. Use exactly one of these forms:
{"kind":"takeoff"}
{"kind":"direct_to","fix":"<FIX>","on_arrival":"land"|"hold"}
{"kind":"heading","degrees":<0-359>,"turn":"left"|"right"|"shortest"}
{"kind":"altitude","height_m":<number>}
{"kind":"speed","speed_mps":<number>}
{"kind":"hold","point":{"at":"present_position"}}  or  {"kind":"hold","point":{"at":"fix","fix":"<FIX>"}}
{"kind":"join_procedure","procedure":"<PROCEDURE>"}
{"kind":"land"}   (land at the present position)
{"kind":"go_around"}
{"kind":"return_to_base"}
{"kind":"unable","reason":"<why>"}   (the message is not a flight instruction, such as a radio check, a frequency change, a transponder code or a weather report)"""


def json_prompt(request):
    envelope = request["envelope"]
    system = (
        "You read one air traffic control style instruction for a small drone and turn it into a directive.\n"
        + JSON_FORMS
        + f"\nPermitted fixes: {', '.join(envelope['fixes'])}. Permitted procedures: {', '.join(envelope['procedures']) or 'none'}."
        + f"\nHeight is {envelope['height_m']['min']} to {envelope['height_m']['max']} metres. Speed is {envelope['speed_mps']['min']} to {envelope['speed_mps']['max']} metres per second."
        + "\n\"turn\" is \"shortest\" when the message names no side."
        + f"\n{request['legend']}"
    )
    return system, f"INSTRUCTION: {request['message']}"


def first_json_object(text):
    start = text.find("{")
    if start < 0:
        raise ValueError(f"no JSON object in the model output: {text[:120]!r}")
    depth = 0
    for index in range(start, len(text)):
        depth += {"{": 1, "}": -1}.get(text[index], 0)
        if depth == 0:
            return json.loads(text[start:index + 1])
    raise ValueError(f"the JSON object does not close: {text[:120]!r}")
