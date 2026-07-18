// Checks for the viewer's video-diagnostics decision logic: readiness is
// logged once per transition (not per frame), and read failures log on the
// first occurrence then coalesce to one line per interval.
//
// Run: node clients/web/video-diagnostics.test.mjs

import { readinessTransition, shouldLogReadFailure } from "./video-diagnostics.js";

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

// ---- read failures log first, then coalesce to one line per interval -------
{
  // The first failure of a session (never logged before) logs immediately.
  check("the first failure logs immediately", shouldLogReadFailure(1000, null, 2000) === true);
  // After a log at t=1000, failures inside the interval are suppressed...
  check("a failure within the interval is suppressed", shouldLogReadFailure(1500, 1000, 2000) === false);
  check("a failure just before the interval is suppressed", shouldLogReadFailure(2999, 1000, 2000) === false);
  // ...and one at or past the interval logs again.
  check("a failure exactly at the interval logs", shouldLogReadFailure(3000, 1000, 2000) === true);
  check("a failure past the interval logs", shouldLogReadFailure(5000, 1000, 2000) === true);
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall video diagnostics checks passed");
