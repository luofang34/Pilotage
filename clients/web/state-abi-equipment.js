// State-frame writer for the equipment groups: the JS side of the
// equipment payload codecs in the indicate-instrument-state crate
// (pinned upstream), plus the declared autoflight targets.
//
// These groups describe equipment, not the motion of the aircraft: the
// flight director, the two bearing pointers, the airframe
// configuration, the autoflight modes, and the autoflight targets.
// Their tags sort after every flight-state tag, so state-abi.js appends
// this list to its own. Field codings follow state-abi.js: NaN is
// "absent" for an optional float, and 255 is the fail-closed unknown
// enum byte.

const TAG = Object.freeze({
  FLIGHT_DIRECTOR: 0x0d,
  BEARING_POINTERS: 0x12,
  AIRFRAME_CONFIG: 0x13,
  AP_MODES: 0x14,
  AP_TARGETS: 0x15,
});

const f = (view, off, v) => view.setFloat32(off, v ?? NaN, true);
const b = (view, off, v) => view.setUint8(off, v);

// One pointer is eight bytes. An undeclared source is "none" (0), which
// draws no needle. An undeclared north is unknown (255), which fails
// the group: a needle does not render on a north nobody declared.
function putPointer(view, off, pointer) {
  b(view, off, pointer?.source ?? 0);
  b(view, off + 1, pointer?.reference ?? 255);
  b(view, off + 2, pointer?.valid ? 1 : 0);
  b(view, off + 3, 0);
  f(view, off + 4, pointer?.bearingRad ?? 0);
}

// Each entry is [tag, select, encode], in ascending tag order. The
// encoder writes the group payload at `off` and returns its length.
export const EQUIPMENT_ENCODERS = [
  [
    TAG.FLIGHT_DIRECTOR,
    (s) => s.director,
    (view, off, fd) => {
      b(view, off, fd.mode ?? 255);
      b(view, off + 1, fd.engagement ?? 255);
      b(view, off + 2, 0);
      b(view, off + 3, 0);
      f(view, off + 4, fd.pitchCmdRad);
      f(view, off + 8, fd.rollCmdRad);
      f(view, off + 12, fd.ageMs);
      return 16;
    },
  ],
  [
    TAG.BEARING_POINTERS,
    (s) => s.bearings,
    (view, off, bearings) => {
      putPointer(view, off, bearings.first);
      putPointer(view, off + 8, bearings.second);
      f(view, off + 16, bearings.ageMs);
      return 20;
    },
  ],
  [
    TAG.AIRFRAME_CONFIG,
    (s) => s.airframe,
    (view, off, airframe) => {
      // Sensed and selected flap keep separate slots. A swap would show
      // a detent the pilot chose as a position the airframe reached.
      f(view, off, airframe.flapRatio);
      f(view, off + 4, airframe.flapSelectedRatio);
      f(view, off + 8, airframe.elevatorTrimRatio);
      f(view, off + 12, airframe.aileronTrimRatio);
      f(view, off + 16, airframe.rudderTrimRatio);
      f(view, off + 20, airframe.ageMs);
      return 24;
    },
  ],
  [
    TAG.AP_MODES,
    (s) => s.apModes,
    (view, off, modes) => {
      // An undeclared mode byte is unknown (255). One unknown byte fails
      // the whole group, so no annunciation claims an undeclared mode.
      b(view, off, modes.engagement ?? 255);
      b(view, off + 1, modes.lateralActive ?? 255);
      b(view, off + 2, modes.lateralArmed ?? 255);
      b(view, off + 3, modes.verticalActive ?? 255);
      b(view, off + 4, modes.verticalArmed ?? 255);
      b(view, off + 5, 0);
      b(view, off + 6, 0);
      b(view, off + 7, 0);
      f(view, off + 8, modes.ageMs);
      return 12;
    },
  ],
  [
    TAG.AP_TARGETS,
    (s) => s.apTargets,
    (view, off, targets) => {
      // The layout mirrors the selections group field for field: the
      // altitude target carries the same reference-identity trio.
      f(view, off, targets.airspeedMps);
      b(view, off + 4, targets.altitudeClass ?? 0);
      b(view, off + 5, targets.altitudeModel ?? 0);
      b(view, off + 6, 0);
      b(view, off + 7, 0);
      f(view, off + 8, targets.altitudeM);
      view.setUint32(off + 12, targets.altitudeOriginId ?? 0, true);
      f(view, off + 16, targets.verticalSpeedMps);
      return 20;
    },
  ],
];

// One state fragment that holds every equipment group. The frame-size
// probe in state-abi.js spreads it. Presence sizes the frame; the
// values do not.
export const EQUIPMENT_PROBE_STATE = Object.freeze({
  director: { ageMs: 0 },
  bearings: { ageMs: 0 },
  airframe: { ageMs: 0 },
  apModes: { ageMs: 0 },
  apTargets: {},
});
