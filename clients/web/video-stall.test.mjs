// Frame-arrival stall transitions: enter once at the threshold, never
// repeat while stalled, and recover exactly once on the next frame.

import {
  VIDEO_STALL_THRESHOLD_MS,
  createVideoFreshness,
  newlyStalledSources,
  noteVideoFrame,
} from "./video-stall.js";

let failures = 0;
function check(name, ok) {
  if (ok) {
    console.log(`ok   - ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL - ${name}`);
  }
}

const f = createVideoFreshness();
check("no sources, no stalls", newlyStalledSources(f, 10_000).length === 0);

noteVideoFrame(f, 0, 1000);
noteVideoFrame(f, 2, 1000);
check("fresh frames do not stall", newlyStalledSources(f, 1000 + VIDEO_STALL_THRESHOLD_MS - 1).length === 0);

const entered = newlyStalledSources(f, 1000 + VIDEO_STALL_THRESHOLD_MS);
check("both silent sources cross the threshold together", entered.length === 2);
check("a stalled source reports its transition exactly once", newlyStalledSources(f, 60_000).length === 0);

check("a frame recovers a stalled source (transition reported)", noteVideoFrame(f, 0, 61_000) === true);
check("a frame on a fresh source is no transition", noteVideoFrame(f, 0, 61_005) === false);
check("the other source stays stalled without re-reporting", newlyStalledSources(f, 61_010).length === 0);
check("the recovered source can stall again later", newlyStalledSources(f, 61_005 + VIDEO_STALL_THRESHOLD_MS).length === 1);

if (failures > 0) {
  console.error(`${failures} failure(s)`);
  process.exit(1);
}
console.log("all video-stall transition checks passed");
