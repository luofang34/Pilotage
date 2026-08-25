// The camera vocabulary the map controls read, pinned against the Apple
// client's SituationCamera and MapControlsView. A reader meets the same
// rule on both clients, so a threshold or a wording changed on one side
// fails here until the other side follows.
//
// Run: node clients/web/situation-camera.test.mjs

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import {
  CAMERA_MOVE_DURATION_MS,
  LEVEL_CONTROL_LABEL,
  cardinal,
  headingControlLabel,
  normalizeHeadingDeg,
  situationCamera,
  spokenHeading,
} from "./situation-camera.js";

const webRoot = dirname(fileURLToPath(import.meta.url));
const binding = join(
  webRoot,
  "..",
  "apple",
  "Packages",
  "PilotageMapLibreBinding",
  "Sources",
  "PilotageMapLibreBinding",
);
const cameraSwift = readFileSync(join(binding, "SituationCamera.swift"), "utf8");
const mapViewSwift = readFileSync(join(binding, "SituationMapView.swift"), "utf8");
const controlsSwift = readFileSync(
  join(webRoot, "..", "apple", "App", "MapControlsView.swift"),
  "utf8",
);

function testThresholdsMatchTheAppleCamera() {
  const rotation = cameraSwift.match(
    /isRotated: Bool \{ headingDegrees\.magnitude > ([\d.]+) && headingDegrees\.magnitude < ([\d.]+) \}/,
  );
  const tilt = cameraSwift.match(/isTilted: Bool \{ pitchDegrees > ([\d.]+) \}/);
  assert.ok(rotation && tilt, "SituationCamera.swift declares both thresholds");
  const noticeable = Number(rotation[1]);
  assert.equal(Number(rotation[2]), 360 - noticeable, "the Swift window is symmetric");

  // Just inside the threshold is not a decision, just outside it is.
  assert.equal(situationCamera({ bearingDeg: noticeable - 0.01, pitchDeg: 0 }).isRotated, false);
  assert.equal(situationCamera({ bearingDeg: noticeable + 0.01, pitchDeg: 0 }).isRotated, true);
  assert.equal(situationCamera({ bearingDeg: -(noticeable - 0.01), pitchDeg: 0 }).isRotated, false);
  assert.equal(situationCamera({ bearingDeg: -(noticeable + 0.01), pitchDeg: 0 }).isRotated, true);

  const tiltNoticeable = Number(tilt[1]);
  assert.equal(situationCamera({ bearingDeg: 0, pitchDeg: tiltNoticeable }).isTilted, false);
  assert.equal(situationCamera({ bearingDeg: 0, pitchDeg: tiltNoticeable + 0.01 }).isTilted, true);
}
testThresholdsMatchTheAppleCamera();
console.log("ok - testThresholdsMatchTheAppleCamera");

function testBothBearingConventionsReadTheSame() {
  // MapLibre GL JS normalizes to (-180, 180]; MapLibre Native reports
  // 0 to 360. The same camera must read the same on both.
  for (const [glJs, native] of [
    [-96, 264],
    [-0.2, 359.8],
    [180, 180],
    [0, 0],
  ]) {
    const web = situationCamera({ bearingDeg: glJs, pitchDeg: 0 });
    assert.equal(web.headingDegrees.toFixed(3), normalizeHeadingDeg(native).toFixed(3));
    assert.equal(web.isRotated, native > 0.5 && native < 359.5, `bearing ${glJs}`);
  }
}
testBothBearingConventionsReadTheSame();
console.log("ok - testBothBearingConventionsReadTheSame");

function testCompassWordingMatchesTheAppleControls() {
  const points = controlsSwift.match(
    /let points = \[("N"(?:, "[A-Z]{1,2}")*)\]/,
  );
  assert.ok(points, "MapControlsView.swift declares the compass points");
  const swiftPoints = points[1].split(",").map((name) => name.trim().replaceAll('"', ""));
  for (const [index, name] of swiftPoints.entries()) {
    assert.equal(cardinal(index * 45), name, `${index * 45} degrees is ${name}`);
  }
  // Rounding, not truncation: a heading just past halfway takes the next
  // point, as the Swift `.rounded()` does.
  assert.equal(cardinal(22.6), "NE");
  assert.equal(cardinal(22.4), "N");
  assert.equal(cardinal(359.9), "N", "the wrap point rounds back to north");

  for (const [abbreviation, spoken] of Object.entries({
    N: "north", NE: "north east", E: "east", SE: "south east",
    S: "south", SW: "south west", W: "west", NW: "north west",
  })) {
    assert.ok(
      controlsSwift.includes(`"${abbreviation}": "${spoken}"`),
      `MapControlsView.swift says ${spoken} for ${abbreviation}`,
    );
  }
  assert.equal(spokenHeading(45), "north east");
  assert.equal(headingControlLabel(45), "Facing north east, turn back to north");
  assert.ok(
    controlsSwift.includes('label: "Facing \\(CompassRose.spokenHeading(camera.headingDegrees)), turn back to north"'),
    "the Apple control says the same sentence",
  );
  assert.ok(
    controlsSwift.includes(`label: "${LEVEL_CONTROL_LABEL}"`),
    "the Apple level control says the same words",
  );
}
testCompassWordingMatchesTheAppleControls();
console.log("ok - testCompassWordingMatchesTheAppleControls");

function testCameraMoveDurationMatchesTheAppleBinding() {
  const duration = mapViewSwift.match(/cameraMoveDuration[^=]*= ([\d.]+)/);
  assert.ok(duration, "SituationMapView.swift declares the camera move duration");
  assert.equal(CAMERA_MOVE_DURATION_MS, Math.round(Number(duration[1]) * 1000));
}
testCameraMoveDurationMatchesTheAppleBinding();
console.log("ok - testCameraMoveDurationMatchesTheAppleBinding");

console.log("\nall situation camera checks passed");
