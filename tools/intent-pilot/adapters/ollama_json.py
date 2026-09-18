"""JSON from a model that an Ollama server runs. Set OLLAMA_MODEL and OLLAMA_URL."""

import json
import os
import sys
import urllib.request

from adapter_common import ALL_KINDS, clean_directive, first_json_object, json_prompt, protect_stdout, serve

MODEL = os.environ.get("OLLAMA_MODEL", "gemma4:e4b")
URL = os.environ.get("OLLAMA_URL", "http://127.0.0.1:11434") + "/api/chat"


def chat(system, user):
    body = json.dumps({"model": MODEL, "stream": False, "format": "json", "think": False,
                       "options": {"temperature": 0, "num_predict": 128},
                       "messages": [{"role": "system", "content": system},
                                    {"role": "user", "content": user}]}).encode()
    request = urllib.request.Request(URL, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=110) as response:
        return json.loads(response.read())["message"]["content"]


def answer(request):
    system, user = json_prompt(request)
    return clean_directive(first_json_object(chat(system, user)), request["envelope"]), {}


if __name__ == "__main__":
    PORT = protect_stdout()
    chat("Reply with {}.", "ready?")  # loads the model before the declaration
    serve(PORT, {"adapter": "ollama-json/1", "model": MODEL, "kinds": ALL_KINDS,
                 "frames": {"max_frames": 0, "projections": []}}, answer)
    sys.exit(0)
