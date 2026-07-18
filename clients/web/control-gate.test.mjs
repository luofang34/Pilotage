// The input-loss gate (CTRL-04, #147): blur latches synchronously, so the
// polling failure modes — blur/refocus between ticks, blur during an await
// (e.g. writer.ready), a held arm button edging after a missed blur — are
// structurally impossible, and no frame is published under a latched
// generation.
//
// Run: node clients/web/control-gate.test.mjs

import { createControlGate } from "./control-gate.js";
import { risingArmEdges } from "./control-edges.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

// --- blur/refocus entirely between ticks cannot be missed -------------------
{
  let focused = true;
  const gate = createControlGate({ isFocused: () => focused });
  check("focused and unlatched publishes", gate.mayPublish() === true);

  // Blur fires between ticks; focus returns before the next tick runs.
  focused = false;
  gate.latchInputLoss();
  focused = true;
  check(
    "a blur/refocus between ticks still relinquishes (latched, not polled)",
    gate.mayPublish() === false,
  );
  check("refocus alone never clears the latch", gate.isLatched() === true);
}

// --- blur during an await (writer.ready): no frame after the await ---------
{
  let focused = true;
  const gate = createControlGate({ isFocused: () => focused });
  const events = [];
  // The loop checked the gate, parked on an await, and the blur landed
  // while it was parked; the post-await re-check must refuse the write.
  const tick = async () => {
    if (!gate.mayPublish()) return events.push("relinquish-at-top");
    await Promise.resolve().then(() => {
      focused = false;
      gate.latchInputLoss(); // blur event fires during the await
      focused = true; // and focus even returns before the loop resumes
    });
    if (!gate.mayPublish()) return events.push("refused-post-await");
    events.push("frame-sent");
  };
  await tick();
  check("blur during an await refuses the write", events.join(",") === "refused-post-await");
  await tick();
  check("the following tick relinquishes at the top", events.join(",").endsWith("relinquish-at-top"));
}

// --- no post-latch frames, ever, under the same generation ------------------
{
  const gate = createControlGate({ isFocused: () => true });
  gate.latchInputLoss();
  let framesSent = 0;
  for (let tick = 0; tick < 100; tick += 1) {
    if (gate.mayPublish()) framesSent += 1;
  }
  check("no frame is publishable after the latch", framesSent === 0);
}

// --- held arm button across a latch cannot edge -----------------------------
{
  // The blur handler clears the previous-pressed set. With a POLLED check
  // a missed blur would leave the loop running, and the held button would
  // read as a fresh rising edge against the cleared set. With the latch,
  // the loop is guaranteed to stop; edges can only be computed again after
  // an explicit reconnect, whose start re-primes the held set — under
  // which a still-held button is NOT a rising edge.
  const gate = createControlGate({ isFocused: () => true });
  const held = new Set(["arm"]);
  let prev = new Set(held); // primed at control start

  gate.latchInputLoss();
  prev = new Set(); // the blur handler clears edge state

  const publishedEdges = [];
  if (gate.mayPublish()) {
    publishedEdges.push(...risingArmEdges(held, prev));
  }
  check("a held button cannot edge after the latch", publishedEdges.length === 0);

  // Explicit reconnect: gate resets AND the held set is re-primed first.
  gate.reset();
  prev = new Set(held);
  const edgesAfterReconnect = gate.mayPublish() ? [...risingArmEdges(held, prev)] : [];
  check("a still-held button is not an edge after reconnect priming", edgesAfterReconnect.length === 0);
}

// --- unfocused-without-an-event still refuses (belt and braces) -------------
{
  let focused = false;
  const gate = createControlGate({ isFocused: () => focused });
  check("actually unfocused refuses even unlatched", gate.mayPublish() === false);
}

if (failures > 0) {
  console.error(`${failures} check(s) failed`);
  process.exit(1);
}
console.log("all control-gate checks passed");
