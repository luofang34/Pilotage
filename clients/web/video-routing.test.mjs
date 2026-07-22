// Browser video-source routing contract. Source identity is shared by the
// legacy v1 frame byte and v2 decoded metadata; both must select the payload
// canvas without taking the unknown-source diagnostic path.

import {
  VIDEO_SOURCE_ID,
  bindVideoTargets,
  resolveVideoTarget,
} from "./video-routing.js";

let failures = 0;
function check(name, condition) {
  if (condition) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

function canvas(id) {
  const context = { id: `${id}-context` };
  return {
    id,
    dataset: {},
    getContext: (kind) => (kind === "2d" ? context : null),
  };
}

const canvases = new Map([
  ["video", canvas("video")],
  ["chaseVideo", canvas("chaseVideo")],
  ["gimbalVideo", canvas("gimbalVideo")],
]);
const documentRoot = { getElementById: (id) => canvases.get(id) ?? null };
const targets = bindVideoTargets(documentRoot);

check("FPV source 0 binds the onboard canvas", targets[VIDEO_SOURCE_ID.FPV].canvas.id === "video");
check(
  "chase source 1 binds a distinct canvas",
  targets[VIDEO_SOURCE_ID.CHASE].canvas.id === "chaseVideo",
);
check(
  "payload source 2 binds the gimbal canvas",
  targets[VIDEO_SOURCE_ID.GIMBAL].canvas.id === "gimbalVideo",
);
check(
  "runtime registration marks the payload canvas as source 2",
  canvases.get("gimbalVideo").dataset.videoSourceId === "2",
);

const unknown = [];
const v1Body = Uint8Array.of(VIDEO_SOURCE_ID.GIMBAL);
const v2Meta = { sourceId: VIDEO_SOURCE_ID.GIMBAL };
check(
  "a v1 source-2 byte resolves to the payload canvas",
  resolveVideoTarget(targets, v1Body[0], (sourceId) => unknown.push(sourceId)).canvas.id ===
    "gimbalVideo",
);
check(
  "v2 source-2 metadata resolves to the payload canvas",
  resolveVideoTarget(targets, v2Meta.sourceId, (sourceId) => unknown.push(sourceId)).canvas.id ===
    "gimbalVideo",
);
check("source 2 never takes the unknown-source path", unknown.length === 0);
check(
  "an unassigned source still takes the unknown-source path",
  resolveVideoTarget(targets, 3, (sourceId) => unknown.push(sourceId)) === null && unknown[0] === 3,
);

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall video routing checks passed");
