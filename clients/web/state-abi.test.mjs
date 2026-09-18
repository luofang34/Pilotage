// Cross-language pin for the state-frame ABI writer.
//
// Run: node clients/web/state-abi.test.mjs
//
// The committed golden frames in the indicate-instrument-state crate's
// fixtures/ directory (the codec owner's own tree, so they travel with
// the crate and arrive here at the pinned upstream rev) are generated
// by the upstream `cargo xtask gen-state-fixture` from the shared Rust
// posture fixtures and pinned by the Rust codec's own tests. This suite
// rebuilds the same three states as writer input objects and requires
// byte equality, so any drift between state-abi.js and the Rust codec —
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
  const path = join(crateDir("indicate-instrument-state"), "fixtures", `${stem}.hex`);
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
// TurnBasis HeadingRate=0/TrackRate=1, NavScale Enroute=0/Terminal=1/
// Approach=2, NavSource Nav1=2/Nav2=3, ApEngagement Autopilot=2,
// LateralMode Roll=1/Approach=4, VerticalMode AltitudeCapture=3/
// GlideSlope=6.

const ALL_VALID = {
  attitude: true,
  rates: true,
  position: true,
  velocityHorizontal: true,
  velocityVertical: true,
  heading: true,
  variation: true,
  turn: true,
  slip: true,
  iasTrend: true,
};

function fullState() {
  return {
    attitude: { quat: { w: 0.5, x: 0.5, y: 0.5, z: 0.5 }, rates: [0.02, -0.01, 0.05], ageMs: 80 },
    kinematics: { posNed: [1200, 340, -305], velNed: [52, 9, -2], ageMs: 80 },
    air: { iasMps: 53, baroHpa: 1013.2, tasMps: 58, ageMs: 80 },
    nav: {
      source: 1,
      scale: 1,
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
    dynamics: { turnRps: 0.05, turnBasis: 0, lateralMps2: 0.3, iasTrendMps2: 0.35, ageMs: 85 },
    director: { pitchCmdRad: 0.08, rollCmdRad: -0.2, mode: 1, engagement: 2, ageMs: 80 },
    monitorText: { revision: 9, lines: ["ENG 1 OK", "FUEL 82.5"], ageMs: 500 },
    bearings: {
      first: { source: 2, bearingRad: 1.2, reference: 2, valid: true },
      second: { source: 3, bearingRad: 4.1, reference: 2, valid: true },
      ageMs: 80,
    },
    // Sensed and selected flap differ on purpose: a writer that swaps
    // the two slots cannot match the golden frame.
    airframe: {
      flapRatio: 0.25,
      flapSelectedRatio: 1.0,
      elevatorTrimRatio: -0.2,
      aileronTrimRatio: 0.05,
      ageMs: 80,
    },
    // Five different mode bytes: a writer that swaps two mode slots
    // cannot match the golden frame.
    apModes: {
      engagement: 2,
      lateralActive: 1,
      lateralArmed: 4,
      verticalActive: 6,
      verticalArmed: 3,
      ageMs: 80,
    },
    apTargets: {
      airspeedMps: 61,
      verticalSpeedMps: 2.5,
      altitudeM: 1200,
      altitudeClass: 0,
      altitudeOriginId: 7,
      altitudeModel: 0,
    },
  };
}

function dataGatewayState() {
  return {
    kinematics: { posNed: [-2500, 800, -1200], velNed: [61, -4, 1.5], ageMs: 120 },
    nav: {
      source: 1,
      scale: 1,
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
    valid: { position: true, velocityHorizontal: true, velocityVertical: true },
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
    dynamics: { turnRps: -0.02, turnBasis: 1, lateralMps2: -0.1, iasTrendMps2: 0.35, ageMs: 50 },
  };
}

check("writer version is the pinned wire version", STATE_ABI_VERSION === 8);

for (const [stem, build] of [
  ["state-abi-v8.full", fullState],
  ["state-abi-v8.data-gateway", dataGatewayState],
  ["state-abi-v8.flight-controller", flightControllerState],
]) {
  check(`${stem} matches the committed golden frame byte for byte`, encodedHex(build()) === goldenHex(stem));
}

{
  // Presence is meaning: an empty state is exactly the two-byte header.
  check("an empty state encodes the empty frame", encodedHex({}) === "0800");
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
  check("an all-default trust group encodes as absent", encodedHex({ valid: {} }) === "0800");
  check(
    "explicitly declared defaults still omit the trust group",
    encodedHex({ quality: 255, valid: {}, snapshot: { coherence: 0, generation: 0 } }) === "0800",
  );
  check(
    "one set flag makes the trust group present",
    encodedHex({ valid: { attitude: true } }) !== "0800",
  );
}

{
  // Velocity validity is split: bit 3 (0x0008) is the horizontal
  // north/east pair, bit 8 (0x0100) is vertical speed. The trust flags
  // are the u16 LE at payload offset 2 of the trust group, which starts
  // after the two-byte frame header and the three-byte group header.
  const trustFlags = (valid) => {
    const hex = encodedHex({ valid });
    const bytes = hex.match(/.{2}/g).map((b) => parseInt(b, 16));
    return bytes[7] | (bytes[8] << 8);
  };
  check(
    "horizontal-only velocity validity sets bit 3 and clears bit 8",
    trustFlags({ velocityHorizontal: true }) === 0x0008,
  );
  check(
    "vertical-only velocity validity sets bit 8 and clears bit 3",
    trustFlags({ velocityVertical: true }) === 0x0100,
  );
  check(
    "full-NED velocity validity sets both velocity bits",
    trustFlags({ velocityHorizontal: true, velocityVertical: true }) === 0x0108,
  );
  check(
    "airspeed-trend validity sets bit 9 and no other bit",
    trustFlags({ iasTrend: true }) === 0x0200,
  );
}

{
  // The decoder refuses the whole frame when a group is shorter than its
  // minimum. The appended fields must therefore be on the wire even when
  // the source does not supply them. Frame: [ver][count][tag][len lo]
  // [len hi][payload...].
  const payload = (state) => {
    const bytes = encodedHex(state).match(/.{2}/g).map((b) => parseInt(b, 16));
    return { len: bytes[3] | (bytes[4] << 8), bytes: bytes.slice(5) };
  };
  const air = payload({ air: { iasMps: 40, ageMs: 10 } });
  check(
    "an air group with no true airspeed writes 16 bytes with a NaN tail",
    air.len === 16 && air.bytes.slice(12, 16).join(",") === "0,0,192,127",
  );
  const nav = payload({ nav: { source: 1, ageMs: 10 } });
  check(
    "a nav group with no declared scale writes 43 bytes and the unknown scale",
    nav.len === 43 && nav.bytes[42] === 0xff,
  );
  const dyn = payload({ dynamics: { turnBasis: 0, turnRps: 0.1, ageMs: 10 } });
  check(
    "a dynamics group with no airspeed trend writes 20 bytes with a NaN tail",
    dyn.len === 20 && dyn.bytes.slice(16, 20).join(",") === "0,0,192,127",
  );
}

{
  // An undeclared bearing-pointer north and an undeclared autoflight mode
  // must reach the wire as the unknown byte, so the Rust side fails the
  // group and draws nothing.
  const bytesOf = (state) =>
    encodedHex(state).match(/.{2}/g).map((b) => parseInt(b, 16)).slice(5);
  const bearings = bytesOf({ bearings: { first: { source: 2, bearingRad: 1, valid: true }, ageMs: 5 } });
  check(
    "an undeclared bearing reference encodes the unknown byte",
    bearings[1] === 0xff && bearings[9] === 0xff,
  );
  check("an absent second pointer encodes source none", bearings[8] === 0);
  const modes = bytesOf({ apModes: { engagement: 2, ageMs: 5 } });
  check(
    "undeclared autoflight modes encode the unknown byte",
    modes[1] === 0xff && modes[2] === 0xff && modes[3] === 0xff && modes[4] === 0xff,
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
