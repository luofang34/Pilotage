// Checks for the viewer's connection auto-recovery decisions: exponential
// backoff capped at a ceiling, and reconnect only when the user wants it, no
// session is active, the tab is visible, and the attempt budget is left.
//
// Run: node clients/web/session-recovery.test.mjs

import { reconnectDelayMs, reconnectDecision } from "./session-recovery.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

// ---- backoff is exponential and capped -------------------------------------
{
  const opts = { baseMs: 1000, maxMs: 15000 };
  check("first attempt waits the base delay", reconnectDelayMs(0, opts) === 1000);
  check("second attempt doubles", reconnectDelayMs(1, opts) === 2000);
  check("third attempt doubles again", reconnectDelayMs(2, opts) === 4000);
  check("delay is capped at maxMs", reconnectDelayMs(10, opts) === 15000);
  check("the cap is never exceeded", reconnectDelayMs(100, opts) === 15000);
}

// ---- reconnect only under the right conditions -----------------------------
{
  const base = { wanted: true, active: false, visible: true, attempts: 0, maxAttempts: 6 };
  const attempt = (o) => reconnectDecision({ ...base, ...o });

  check("wanted + inactive + visible + budget => attempt", attempt({}).attempt === true);
  check("no attempt when the user never connected", attempt({ wanted: false }).attempt === false);
  check("no attempt while a session is already active", attempt({ active: true }).attempt === false);
  check("no attempt while the tab is hidden (would just drop again)", attempt({ visible: false }).attempt === false);

  // The budget is a hard stop.
  check("attempts under the cap do not give up", attempt({ attempts: 5 }).giveUp === false);
  const spent = attempt({ attempts: 6 });
  check("attempts at the cap give up", spent.giveUp === true && spent.attempt === false);
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall session recovery checks passed");
