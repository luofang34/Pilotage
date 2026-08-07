// Cross-language pin for the state-frame ABI v6 writer.
//
// Run: node clients/web/state-abi.test.mjs
//
// The committed golden frames in the pilotage-instrument-state crate's
// fixtures/ directory (the codec owner's own tree, so they travel with
// the crate and arrive here at the pinned upstream rev) are generated
// by the upstream `cargo xtask gen-state-fixture` from the shared Rust
// posture fixtures and pinned by the Rust codec's own tests. This suite
// rebuilds the same three states as writer input objects and requires
// byte equality, so any drift between state-abi.js and abi/v6.rs —
// offsets, widths, endianness, enum codings, presence rules, ident
// atoms — turns CI red on whichever side moved.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { crateDir } from "./crate-dir.mjs";
import { STATE_ABI_VERSION, encodeState } from "./state-abi.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

function goldenHex(stem) {
  const path = join(crateDir("pilotage-instrument-state"), "fixtures", `${stem}.hex`);
  return readFileSync(path, "utf8").trim();
}

function encodedHex(state) {
  const buffer = new ArrayBuffer(1024);
  const len = encodeState(new DataView(buffer), state);
  return [...new Uint8Array(buffer, 0, len)]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

// Enum byte codings mirror the Rust wire vocabulary: NavSource Gps=1,
// NavFromTo To=1, HeadingReference Magnetic=0/True=1/SimLocalTrue=2,
// AltitudeClass LocalRelative=0/BaroIndicated=1/Pressure=2,
// SnapshotCoherence Insufficient=0/Coherent=1, EstimateQuality Good=0,
// TurnBasis HeadingRate=0/TrackRate=1.

const ALL_VALID = {
  attitude: true,
  rates: true,
  position: true,
  velocity: true,
  heading: true,
  variation: true,
  turn: true,
  slip: true,
};

function fullState() {
  return {
    attitude: { quat: { w: 0.5, x: 0.5, y: 0.5, z: 0.5 }, rates: [0.02, -0.01, 0.05], ageMs: 80 },
    kinematics: { posNed: [1200, 340, -305], velNed: [52, 9, -2], ageMs: 80 },
    air: { iasMps: 53, baroHpa: 1013.2, ageMs: 80 },
    nav: {
      source: 1,
      fromto: 1,
      courseRad: 0.6,
      cdiDots: 0.7,
      vdevDots: -0.4,
      distNm: 12.4,
      courseReference: 2,
      toIdent: "WPT-2",
      fromIdent: "KMRY",
      ageMs: 80,
    },
    wind: { fromRad: 2.1, speedMps: 7.5, ageMs: 80 },
    selections: {
      headingBugRad: 1.0,
      headingBugReference: 2,
      altitudeSelM: 500,
      altitudeSelClass: 0,
      altitudeSelOriginId: 7,
      altitudeSelModel: 0,
      baroSelHpa: 1013.2,
    },
    quality: 0,
    valid: ALL_VALID,
    snapshot: { generation: 42, coherence: 1 },
    altitude: { referenceClass: 1, sampleM: 950, geoidModel: 0, originId: 7 },
    heading: { rad: 0.35, reference: 2, ageMs: 90 },
    variation: { eastRad: 0.15, sourceId: 3, ageMs: 120 },
    dynamics: { turnRps: 0.05, turnBasis: 0, lateralMps2: 0.3, ageMs: 85 },
    director: { pitchCmdRad: 0.08, rollCmdRad: -0.2, mode: 1, engagement: 2, ageMs: 80 },
    monitorText: { revision: 9, lines: ["ENG 1 OK", "FUEL 82.5"], ageMs: 500 },
  };
}

function dataGatewayState() {
  return {
    kinematics: { posNed: [-2500, 800, -1200], velNed: [61, -4, 1.5], ageMs: 120 },
    nav: {
      source: 1,
      fromto: 1,
      courseRad: 1.2,
      cdiDots: -0.3,
      distNm: 8.7,
      courseReference: 1,
      toIdent: "WPT-3",
      fromIdent: "GATE-A",
      ageMs: 150,
    },
    quality: 0,
    valid: { position: true, velocity: true },
    snapshot: { generation: 7, coherence: 0 },
    altitude: { referenceClass: 2, sampleM: 1150, geoidModel: 0, originId: 0 },
  };
}

function flightControllerState() {
  return {
    attitude: { quat: { w: 1, x: 0, y: 0, z: 0 }, rates: [0.01, 0, -0.02], ageMs: 40 },
    kinematics: { posNed: [10, -20, -80], velNed: [21, 3, -0.5], ageMs: 40 },
    air: { iasMps: 39, baroHpa: 1020.5, ageMs: 45 },
    wind: { fromRad: 0.8, speedMps: 4.2, ageMs: 200 },
    selections: {
      headingBugRad: 2.4,
      headingBugReference: 0,
      altitudeSelClass: 0,
      altitudeSelOriginId: 0,
      altitudeSelModel: 0,
      baroSelHpa: 1020.5,
    },
    quality: 0,
    valid: ALL_VALID,
    snapshot: { generation: 991, coherence: 1 },
    altitude: { referenceClass: 1, sampleM: 320, geoidModel: 0, originId: 0 },
    heading: { rad: 1.9, reference: 0, ageMs: 60 },
    variation: { eastRad: -0.05, sourceId: 2, ageMs: 60 },
    dynamics: { turnRps: -0.02, turnBasis: 1, lateralMps2: -0.1, ageMs: 50 },
  };
}

check("writer version is the v6 wire version", STATE_ABI_VERSION === 6);

for (const [stem, build] of [
  ["state-abi-v6.full", fullState],
  ["state-abi-v6.data-gateway", dataGatewayState],
  ["state-abi-v6.flight-controller", flightControllerState],
]) {
  check(`${stem} matches the committed golden frame byte for byte`, encodedHex(build()) === goldenHex(stem));
}

{
  // Presence is meaning: an empty state is exactly the two-byte header.
  check("an empty state encodes the empty frame", encodedHex({}) === "0600");
}

{
  // A malformed ident must reach the wire as the INVALID marker (0xff
  // length), never as truncated or partial text.
  const hex = encodedHex({ nav: { source: 1, toIdent: "wpt", ageMs: 10 } });
  const bytes = hex.match(/.{2}/g).map((b) => parseInt(b, 16));
  // Frame: [ver][count][tag][len lo][len hi][payload...]; to_ident len
  // byte sits at payload offset 24.
  check("an out-of-charset ident encodes the INVALID marker", bytes[5 + 24] === 0xff);
  const over = encodedHex({ nav: { source: 1, toIdent: "ABCDEFGHI", ageMs: 10 } });
  const overBytes = over.match(/.{2}/g).map((b) => parseInt(b, 16));
  check("an over-length ident encodes the INVALID marker", overBytes[5 + 24] === 0xff);
}

{
  // Canonicalization: a trust group whose quality, flags, and snapshot
  // all equal their fail-closed defaults encodes as absent — matching
  // the Rust encoder, so equal states produce equal bytes.
  check("an all-default trust group encodes as absent", encodedHex({ valid: {} }) === "0600");
  check(
    "explicitly declared defaults still omit the trust group",
    encodedHex({ quality: 255, valid: {}, snapshot: { coherence: 0, generation: 0 } }) === "0600",
  );
  check(
    "one set flag makes the trust group present",
    encodedHex({ valid: { attitude: true } }) !== "0600",
  );
}

{
  // A non-string ident is malformed content, not empty text.
  const hex = encodedHex({ nav: { source: 1, toIdent: 12345, ageMs: 10 } });
  const bytes = hex.match(/.{2}/g).map((b) => parseInt(b, 16));
  check("a non-string ident encodes the INVALID marker", bytes[5 + 24] === 0xff);
}

{
  // An over-long monitor channel is refused, never silently truncated
  // (AIR-IN-014) — matching MonitorText::new on the Rust side.
  let refused = false;
  try {
    encodedHex({ monitorText: { revision: 1, lines: Array(9).fill("A"), ageMs: 0 } });
  } catch (error) {
    refused = error instanceof RangeError;
  }
  check("more than eight monitor lines throws instead of truncating", refused);
}

if (failures > 0) {
  console.error(`${failures} check(s) failed`);
  process.exit(1);
}
console.log("state-abi golden checks passed");
