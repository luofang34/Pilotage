// Frame-arrival stall transitions: enter once at the threshold, never
// repeat while stalled, and recover exactly once on the next frame.

import {
  VIDEO_STALL_THRESHOLD_MS,
  allVideoSourcesStalled,
  createVideoFreshness,
  newlyStalledSources,
  noteVideoFrame,
} from "./video-stall.js";
import {
  bandwidthBannerText,
  normalizeVideoDelivery,
  stallWatchEnabled,
} from "./video-bandwidth.js";

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
check("no observed sources is not an all-source stall", allVideoSourcesStalled(f) === false);

noteVideoFrame(f, 0, 1000);
noteVideoFrame(f, 2, 1000);
check("fresh frames do not stall", newlyStalledSources(f, 1000 + VIDEO_STALL_THRESHOLD_MS - 1).length === 0);

const entered = newlyStalledSources(f, 1000 + VIDEO_STALL_THRESHOLD_MS);
check("both silent sources cross the threshold together", entered.length === 2);
check("every observed source is now stalled", allVideoSourcesStalled(f) === true);
check("a stalled source reports its transition exactly once", newlyStalledSources(f, 60_000).length === 0);

check("a frame recovers a stalled source (transition reported)", noteVideoFrame(f, 0, 61_000) === true);
check("one recovered source clears all-source stall", allVideoSourcesStalled(f) === false);
check("a frame on a fresh source is no transition", noteVideoFrame(f, 0, 61_005) === false);
check("the other source stays stalled without re-reporting", newlyStalledSources(f, 61_010).length === 0);
check("the recovered source can stall again later", newlyStalledSources(f, 61_005 + VIDEO_STALL_THRESHOLD_MS).length === 1);

const degraded = normalizeVideoDelivery({
  mode: "degraded",
  reason: "bandwidth",
  budgetBytesPerSecond: 1_000_000,
});
check(
  "degradation names bandwidth and the enacted aggregate budget",
  bandwidthBannerText(degraded) === "video degraded — bandwidth (8.0 Mbit/s)",
);
check("degraded video still participates in freshness checks", stallWatchEnabled(degraded));

const suspended = normalizeVideoDelivery({
  mode: "suspended",
  reason: "bandwidth",
  budgetBytesPerSecond: 0,
});
check(
  "bandwidth suspension is distinct from a no-frame stall",
  bandwidthBannerText(suspended) === "video suspended — bandwidth"
    && !stallWatchEnabled(suspended),
);
check(
  "an unknown delivery mode fails visibly closed",
  normalizeVideoDelivery({ mode: "future-mode" }).mode === "suspended",
);

if (failures > 0) {
  console.error(`${failures} failure(s)`);
  process.exit(1);
}
console.log("all video-stall transition checks passed");
