// Checks for the viewer's video-diagnostics decision logic: readiness is
// logged once per transition (not per frame), read failures coalesce, and a
// supervised reader resumes on a transient exit but gives up after repeated
// immediate failures.
//
// Run: node clients/web/video-diagnostics.test.mjs

import {
  readinessTransition,
  shouldLogReadFailure,
  restartVerdict,
} from "./video-diagnostics.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

// ---- readiness is a per-transition event, not a per-frame one --------------
{
  // First ever frame while not ready: log once, remember the reason.
  const first = readinessTransition(undefined, false, "mapping-unavailable");
  check("first not-ready frame logs", first !== null);
  check(
    "not-ready message names the reason",
    first.message === "not conformal-ready: mapping-unavailable",
  );
  check("remembered state is the reason", first.state === "mapping-unavailable");

  // Every subsequent frame in the same state: nothing to log (the spam fix).
  check(
    "an unchanged not-ready state does not re-log",
    readinessTransition("mapping-unavailable", false, "mapping-unavailable") === null,
  );

  // Becoming ready is a transition worth one line.
  const recovered = readinessTransition("mapping-unavailable", true, "mapping-unavailable");
  check("recovery to ready logs once", recovered !== null && recovered.state === null);
  check("recovery message is now-ready", recovered.message === "now conformal-ready");
  check(
    "staying ready does not re-log",
    readinessTransition(null, true, "mapping-unavailable") === null,
  );

  // A different not-ready reason is a new transition.
  const changed = readinessTransition("mapping-unavailable", false, "no-snapshot");
  check("a changed reason logs again", changed !== null && changed.state === "no-snapshot");
}

// ---- read failures coalesce to one line per interval -----------------------
{
  check("the first failure logs (last-logged 0)", shouldLogReadFailure(1000, 0, 2000) === false);
  check("a failure past the interval logs", shouldLogReadFailure(3000, 1000, 2000) === true);
  check("a failure within the interval is suppressed", shouldLogReadFailure(1500, 1000, 2000) === false);
  check("a failure exactly at the interval logs", shouldLogReadFailure(3000, 1000, 2000) === true);
}

// ---- supervised-reader restart verdict -------------------------------------
{
  const opts = { minUptimeMs: 1000, maxImmediate: 5 };

  // A reader that ran a long time before exiting is a transient interruption:
  // resume, and the immediate-exit counter resets.
  const transient = restartVerdict(5000, 3, opts);
  check("a long-lived reader resumes", !transient.giveUp && !transient.ranBriefly);
  check("a transient exit clears the immediate counter", transient.immediateExits === 0);

  // A reader that exits immediately increments the counter but keeps resuming
  // until the cap.
  let exits = 0;
  let gaveUp = false;
  for (let i = 0; i < 5; i += 1) {
    const v = restartVerdict(10, exits, opts);
    exits = v.immediateExits;
    gaveUp = v.giveUp;
  }
  check("five immediate exits reach give-up", gaveUp && exits === 5);
  check(
    "the fourth immediate exit still resumes",
    restartVerdict(10, 3, opts).giveUp === false,
  );
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall video diagnostics checks passed");
