// Tests for the H.264 Annex-B classification helpers that drive the WebCodecs
// decode groundwork: keyframe detection and codec-string derivation over raw
// NAL units. The VideoDecoder path itself needs a browser and a real H.264
// stream, so only the pure, host-runnable classification is covered here.

import { forEachNalType, isKeyframe, avcCodecString } from "./video-h264.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

// nal(type, ...body) with a 4-byte start code; sps() carries profile/constraint/level.
function nal(type, ...body) {
  return [0, 0, 0, 1, type & 0x1f, ...body];
}
function sps(profile, constraint, level) {
  return nal(7, profile, constraint, level);
}

// A keyframe access unit: SPS + PPS + IDR slice.
{
  const au = new Uint8Array([...sps(0x42, 0xe0, 0x1e), ...nal(8), ...nal(5, 9, 9, 9)]);
  const types = [];
  forEachNalType(au, (t) => types.push(t));
  check("iterates every NAL type in order", types.join(",") === "7,8,5");
  check("IDR slice marks a keyframe", isKeyframe(au) === true);
  check("codec string is avc1 from the SPS bytes", avcCodecString(au) === "avc1.42e01e");
}

// A delta access unit: a non-IDR slice, no SPS.
{
  const au = new Uint8Array(nal(1, 4, 4));
  check("non-IDR access unit is not a keyframe", isKeyframe(au) === false);
  check("no SPS yields no codec string", avcCodecString(au) === null);
}

// The 3-byte start code (0x000001) is recognized as well as the 4-byte one.
{
  const au = new Uint8Array([0, 0, 1, 5, 1, 2, 3]);
  check("3-byte start code is recognized", isKeyframe(au) === true);
}

// Garbage without any start code yields no NAL units (fail closed).
{
  const au = new Uint8Array([9, 9, 9, 9, 9]);
  let count = 0;
  forEachNalType(au, () => (count += 1));
  check("no start code yields no NAL units", count === 0);
  check("garbage is not a keyframe", isKeyframe(au) === false);
}

console.log(failures === 0 ? "\nall H.264 classification checks passed" : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
