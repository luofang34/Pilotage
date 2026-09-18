"""A model adapter that reads perfectly: it answers with the directive that the scenario
author wrote for the message. Usage: oracle.py <scenario.json>

It separates the flight and the verifier from the quality of a model. A scenario that
fails with this adapter has a defect in the scenario, the executor or the vehicle.
"""

import json
import sys

from adapter_common import ALL_KINDS, protect_stdout, serve, unable

if __name__ == "__main__":
    PORT = protect_stdout()
    MEANS = {message["text"]: message["means"] for message in json.load(open(sys.argv[1]))["messages"]}
    serve(PORT, {"adapter": "oracle/1", "model": "none (the scenario author's directives)", "kinds": ALL_KINDS,
                 "frames": {"max_frames": 0, "projections": []}},
          lambda request: (MEANS.get(request["message"], unable("the scenario has no such message")), {}))
    sys.exit(0)
