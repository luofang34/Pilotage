"""A model adapter with no model: regular expressions.

It is the floor for every comparison, and it tests the harness without a GPU. Its
word lists come from the open suite only. Do not add a word because a held-out case
uses it.
"""

import re
import sys

from adapter_common import protect_stdout, serve, unable

WORDS = {"zero": "0", "one": "1", "two": "2", "three": "3", "four": "4", "five": "5",
         "six": "6", "seven": "7", "eight": "8", "nine": "9", "niner": "9"}


def answer(request):
    text = request["message"].lower()
    envelope = request["envelope"]
    spoken = " ".join(WORDS.get(word.strip(".,"), word) for word in text.split())
    fixes = [fix for fix in envelope["fixes"] if re.search(rf"\b{fix.lower()}\b", text)]
    arrival = "land" if re.search(r"\bland\b", text) else "hold"
    number = re.search(r"(\d+(?:\.\d+)?)", text)
    if re.search(r"\bgo around\b", text):
        return {"kind": "go_around"}, {}
    for procedure in envelope["procedures"]:
        letters, digits = re.match(r"([A-Z]+)(\d+)", procedure).groups()
        squeezed = re.sub(r"(?<=\d) (?=\d)", "", spoken)
        if letters.lower() in text and (digits in squeezed.replace(" 0", " 0") or digits.lstrip("0") in squeezed):
            return {"kind": "join_procedure", "procedure": procedure}, {}
    if re.search(r"\b(return to base|back home)\b", text):
        return {"kind": "return_to_base"}, {}
    if "heading" in text:
        digits = re.search(r"(\d(?: ?\d){1,2})", spoken)
        if not digits:
            return unable("no heading number"), {}
        turn = "left" if "left" in text else "right" if "right" in text else "shortest"
        return {"kind": "heading", "degrees": int(digits.group(1).replace(" ", "")) % 360, "turn": turn}, {}
    if re.search(r"\b(climb|descend|maintain \d+ metres)\b", text) and number and "per second" not in text:
        return {"kind": "altitude", "height_m": float(number.group(1))}, {}
    if "per second" in text and number:
        return {"kind": "speed", "speed_mps": float(number.group(1))}, {}
    if re.search(r"\bhold\b", text) and not re.search(r"\b(direct|go to|fly to|proceed)\b", text):
        point = {"at": "fix", "fix": fixes[-1]} if fixes else {"at": "present_position"}
        return {"kind": "hold", "point": point}, {}
    if fixes and re.search(r"\b(direct|go to|fly to|proceed)\b", text):
        return {"kind": "direct_to", "fix": fixes[-1], "on_arrival": arrival}, {}
    if re.search(r"\btake ?off\b", text):
        return {"kind": "takeoff"}, {}
    if re.search(r"\bland\b", text):
        return {"kind": "land"}, {}
    return unable("no instruction word"), {}


if __name__ == "__main__":
    PORT = protect_stdout()
    serve(PORT, {"adapter": "keyword-baseline/1", "model": "none (regular expressions)",
                 "kinds": ["takeoff", "direct_to", "heading", "altitude", "speed", "hold",
                           "join_procedure", "land", "go_around", "return_to_base", "unable"],
                 "frames": {"max_frames": 0, "projections": []}}, answer)
    sys.exit(0)
