"""Autoregressive JSON from an MLX model. Default: the weights that the RLCD engine uses.

Set MODEL_ID for a different mlx-lm model.
"""

import os
import sys

from adapter_common import ALL_KINDS, clean_directive, first_json_object, json_prompt, protect_stdout, serve

PORT = protect_stdout()

from mlx_lm import generate, load  # noqa: E402

MODEL_ID = os.environ.get("MODEL_ID", "mlx-community/Qwen2.5-1.5B-Instruct-4bit")
MODEL, TOKENIZER = load(MODEL_ID)


def answer(request):
    system, user = json_prompt(request)
    prompt = TOKENIZER.apply_chat_template(
        [{"role": "system", "content": system}, {"role": "user", "content": user}],
        add_generation_prompt=True, tokenize=False)
    text = generate(MODEL, TOKENIZER, prompt=prompt, max_tokens=96, verbose=False)
    return clean_directive(first_json_object(text), request["envelope"]), {}


if __name__ == "__main__":
    serve(PORT, {"adapter": "mlx-json/1", "model": MODEL_ID, "kinds": ALL_KINDS,
                 "frames": {"max_frames": 0, "projections": []}}, answer)
    sys.exit(0)
