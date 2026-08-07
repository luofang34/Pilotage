// State-frame ABI v6 writer: the JS side of the tagged-group contract in
// crates/pilotage-instrument-state/src/abi/v6.rs.
//
// The frame is self-delimiting: [version u8][group count u8] then, per
// present group in strictly ascending tag order, [tag u8][payload length
// u16 LE][payload]. Presence is meaning — a state object key that is
// absent emits no tag, and the Rust decoder resolves that group Missing
// by construction. Field codings mirror the Rust codec exactly: NaN is
// "absent" for optional floats, 255 is the fail-closed unknown enum
// byte, and idents are [len u8][8 bytes zero-padded] from the closed
// charset A-Z 0-9 space '-' (anything else encodes the INVALID marker,
// which fails the nav group visibly on the Rust side).
//
// This writer is pinned byte-for-byte against the committed golden
// frames in crates/pilotage-instrument-state/fixtures/ via
// state-abi.test.mjs; the Rust codec pins against the same files, so
// the two sides of the wasm boundary can only drift by turning CI red.

export const STATE_ABI_VERSION = 6;

const TAG = Object.freeze({
  ATTITUDE: 0x01,
  KINEMATICS: 0x02,
  AIR: 0x03,
  NAV: 0x04,
  WIND: 0x05,
  SELECTIONS: 0x06,
  TRUST: 0x07,
  ALTITUDE: 0x08,
  HEADING: 0x09,
  VARIATION: 0x0a,
  DYNAMICS: 0x0b,
  MONITOR_TEXT: 0x0c,
  FLIGHT_DIRECTOR: 0x0d,
});

const IDENT_CAPACITY = 8;
const TEXT_LINE_CAPACITY = 32;
const TEXT_MAX_LINES = 8;
const TEXT_INVALID_LEN = 0xff;

function identByteOk(code) {
  return (
    (code >= 0x41 && code <= 0x5a) || // A-Z
    (code >= 0x30 && code <= 0x39) || // 0-9
    code === 0x20 || // space
    code === 0x2d // -
  );
}

function textByteOk(code) {
  return identByteOk(code) || code === 0x2e; // .
}

// Writes one length-prefixed, zero-padded text atom of `capacity`
// bytes. Malformed content (over-length or out-of-charset) writes the
// INVALID marker so the Rust side fails the group instead of displaying
// text nobody vetted.
function putTextAtom(view, off, text, capacity, byteOk) {
  const content = text ?? "";
  let invalid = typeof content !== "string" || content.length > capacity;
  for (let i = 0; i < capacity; i += 1) {
    view.setUint8(off + 1 + i, 0);
  }
  if (!invalid) {
    for (let i = 0; i < content.length; i += 1) {
      const code = content.charCodeAt(i);
      if (!byteOk(code)) {
        invalid = true;
        break;
      }
      view.setUint8(off + 1 + i, code);
    }
  }
  if (invalid) {
    for (let i = 0; i < capacity; i += 1) {
      view.setUint8(off + 1 + i, 0);
    }
    view.setUint8(off, TEXT_INVALID_LEN);
  } else {
    view.setUint8(off, content.length);
  }
}

function putIdent(view, off, text) {
  putTextAtom(view, off, text, IDENT_CAPACITY, identByteOk);
}

// Each encoder writes its group payload at `off` and returns the payload
// length. `f` writes an optional float (NaN = absent); `b` a byte.
function groupEncoders() {
  const f = (view, off, v) => view.setFloat32(off, v ?? NaN, true);
  const b = (view, off, v) => view.setUint8(off, v);
  return [
    [
      TAG.ATTITUDE,
      (s) => s.attitude,
      (view, off, att) => {
        f(view, off, att.quat?.w ?? 1);
        f(view, off + 4, att.quat?.x ?? 0);
        f(view, off + 8, att.quat?.y ?? 0);
        f(view, off + 12, att.quat?.z ?? 0);
        f(view, off + 16, att.rates?.[0] ?? 0);
        f(view, off + 20, att.rates?.[1] ?? 0);
        f(view, off + 24, att.rates?.[2] ?? 0);
        f(view, off + 28, att.ageMs);
        return 32;
      },
    ],
    [
      TAG.KINEMATICS,
      (s) => s.kinematics,
      (view, off, kin) => {
        f(view, off, kin.posNed?.[0] ?? 0);
        f(view, off + 4, kin.posNed?.[1] ?? 0);
        f(view, off + 8, kin.posNed?.[2] ?? 0);
        f(view, off + 12, kin.velNed?.[0] ?? 0);
        f(view, off + 16, kin.velNed?.[1] ?? 0);
        f(view, off + 20, kin.velNed?.[2] ?? 0);
        f(view, off + 24, kin.ageMs);
        return 28;
      },
    ],
    [
      TAG.AIR,
      (s) => s.air,
      (view, off, air) => {
        f(view, off, air.iasMps);
        f(view, off + 4, air.baroHpa);
        f(view, off + 8, air.ageMs);
        return 12;
      },
    ],
    [
      TAG.NAV,
      (s) => s.nav,
      (view, off, nav) => {
        b(view, off, nav.source ?? 0);
        b(view, off + 1, nav.fromto ?? 0);
        // Fail-safe default mirrors the Rust codec: an undeclared course
        // reference is unknown (255), which suppresses the CDI/course.
        b(view, off + 2, nav.courseReference ?? 255);
        b(view, off + 3, 0);
        f(view, off + 4, nav.courseRad ?? 0);
        f(view, off + 8, nav.cdiDots ?? 0);
        f(view, off + 12, nav.vdevDots);
        f(view, off + 16, nav.distNm);
        f(view, off + 20, nav.ageMs);
        putIdent(view, off + 24, nav.toIdent);
        putIdent(view, off + 33, nav.fromIdent);
        return 42;
      },
    ],
    [
      TAG.WIND,
      (s) => s.wind,
      (view, off, wind) => {
        f(view, off, wind.fromRad ?? 0);
        f(view, off + 4, wind.speedMps ?? 0);
        f(view, off + 8, wind.ageMs);
        return 12;
      },
    ],
    [
      TAG.SELECTIONS,
      (s) => s.selections,
      (view, off, sel) => {
        f(view, off, sel.headingBugRad ?? 0);
        // An undeclared bug reference is unknown (255): nothing renders
        // on a north nobody declared.
        b(view, off + 4, sel.headingBugReference ?? 255);
        b(view, off + 5, sel.altitudeSelClass ?? 0);
        b(view, off + 6, sel.altitudeSelModel ?? 0);
        b(view, off + 7, 0);
        f(view, off + 8, sel.altitudeSelM);
        view.setUint32(off + 12, sel.altitudeSelOriginId ?? 0, true);
        f(view, off + 16, sel.baroSelHpa);
        return 20;
      },
    ],
    [
      TAG.TRUST,
      // Mirrors the Rust encoder's default-omission: a trust group whose
      // quality, flags, and snapshot all equal their fail-closed defaults
      // encodes as absent, so equal states produce equal bytes on both
      // writers.
      (s) => {
        const v = s.valid ?? {};
        const flags =
          v.attitude || v.rates || v.position || v.velocity ||
          v.heading || v.variation || v.turn || v.slip;
        const snap =
          (s.snapshot?.coherence ?? 0) !== 0 || (s.snapshot?.generation ?? 0) !== 0;
        return (s.quality ?? 255) !== 255 || flags || snap ? s : undefined;
      },
      (view, off, s) => {
        // Undeclared quality is unknown (255, resolves Failed), and
        // validity is never assumed — unset flags mean "not declared
        // valid" (VAL-01).
        b(view, off, s.quality ?? 255);
        b(view, off + 1, s.snapshot?.coherence ?? 0);
        const v = s.valid ?? {};
        const flags =
          (v.attitude ? 0x01 : 0) |
          (v.rates ? 0x02 : 0) |
          (v.position ? 0x04 : 0) |
          (v.velocity ? 0x08 : 0) |
          (v.heading ? 0x10 : 0) |
          (v.variation ? 0x20 : 0) |
          (v.turn ? 0x40 : 0) |
          (v.slip ? 0x80 : 0);
        view.setUint16(off + 2, flags, true);
        view.setUint32(off + 4, s.snapshot?.generation ?? 0, true);
        return 8;
      },
    ],
    [
      TAG.ALTITUDE,
      (s) => s.altitude,
      (view, off, alt) => {
        b(view, off, alt.referenceClass ?? 0);
        b(view, off + 1, alt.geoidModel ?? 0);
        b(view, off + 2, 0);
        b(view, off + 3, 0);
        f(view, off + 4, alt.sampleM);
        view.setUint32(off + 8, alt.originId ?? 0, true);
        return 12;
      },
    ],
    [
      TAG.HEADING,
      (s) => s.heading,
      (view, off, heading) => {
        b(view, off, heading.reference ?? 255);
        b(view, off + 1, 0);
        b(view, off + 2, 0);
        b(view, off + 3, 0);
        f(view, off + 4, heading.rad);
        f(view, off + 8, heading.ageMs);
        return 12;
      },
    ],
    [
      TAG.VARIATION,
      (s) => s.variation,
      (view, off, variation) => {
        b(view, off, variation.sourceId ?? 0);
        b(view, off + 1, 0);
        b(view, off + 2, 0);
        b(view, off + 3, 0);
        f(view, off + 4, variation.eastRad);
        f(view, off + 8, variation.ageMs);
        return 12;
      },
    ],
    [
      TAG.DYNAMICS,
      (s) => s.dynamics,
      (view, off, dyn) => {
        b(view, off, dyn.turnBasis ?? 255);
        b(view, off + 1, 0);
        b(view, off + 2, 0);
        b(view, off + 3, 0);
        f(view, off + 4, dyn.turnRps);
        f(view, off + 8, dyn.lateralMps2);
        f(view, off + 12, dyn.ageMs);
        return 16;
      },
    ],
    [
      TAG.MONITOR_TEXT,
      (s) => s.monitorText,
      (view, off, mt) => {
        // Fixed 274-byte payload: count, reserved, revision u32, eight
        // 33-byte line atoms, age f32 — unused slots stay zero so equal
        // channels produce equal bytes.
        const lines = mt.lines ?? [];
        if (lines.length > TEXT_MAX_LINES) {
          // Mirrors MonitorText::new: an over-long channel is refused,
          // never silently truncated (AIR-IN-014).
          throw new RangeError(`monitor text exceeds ${TEXT_MAX_LINES} lines`);
        }
        b(view, off, lines.length);
        b(view, off + 1, 0);
        view.setUint32(off + 2, mt.revision ?? 0, true);
        const atom = TEXT_LINE_CAPACITY + 1;
        for (let i = 0; i < TEXT_MAX_LINES; i += 1) {
          const at = off + 6 + i * atom;
          if (i < lines.length) {
            putTextAtom(view, at, lines[i], TEXT_LINE_CAPACITY, textByteOk);
          } else {
            for (let z = 0; z < atom; z += 1) view.setUint8(at + z, 0);
          }
        }
        f(view, off + 6 + TEXT_MAX_LINES * atom, mt.ageMs);
        return 6 + TEXT_MAX_LINES * atom + 4;
      },
    ],
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
  ];
}

const ENCODERS = groupEncoders();

// Encodes `state` as a canonical v6 frame into `view` (a DataView over
// the wasm state buffer). Returns the used length, or throws RangeError
// when the buffer cannot hold the frame — the caller surfaces that as a
// state-write failure, never a partial frame.
export function encodeState(view, state) {
  view.setUint8(0, STATE_ABI_VERSION);
  // Zero the count before writing groups: a partial write (buffer too
  // small) then degrades to the empty frame — every group Missing —
  // instead of splicing a new header onto a previous frame's tail.
  view.setUint8(1, 0);
  let count = 0;
  let off = 2;
  for (const [tag, select, encode] of ENCODERS) {
    const group = select(state);
    if (group === undefined || group === null) continue;
    const len = encode(view, off + 3, group);
    view.setUint8(off, tag);
    view.setUint16(off + 1, len, true);
    count += 1;
    off += 3 + len;
  }
  view.setUint8(1, count);
  return off;
}

// A state with every group the writer knows, for measuring the largest
// canonical frame. Values are irrelevant; presence is what sizes it.
const PROBE_STATE = {
  attitude: { quat: { w: 1, x: 0, y: 0, z: 0 }, rates: [0, 0, 0], ageMs: 0 },
  kinematics: { posNed: [0, 0, 0], velNed: [0, 0, 0], ageMs: 0 },
  air: { ageMs: 0 },
  nav: { ageMs: 0 },
  wind: { ageMs: 0 },
  selections: {},
  quality: 0,
  altitude: {},
  heading: { ageMs: 0 },
  variation: { ageMs: 0 },
  dynamics: { ageMs: 0 },
  monitorText: { revision: 0, lines: [], ageMs: 0 },
};

// The largest frame this writer can produce, measured by encoding the
// probe rather than mirrored as a hand-maintained constant. A state
// buffer smaller than this cannot hold a full canonical frame.
export function maxFrameBytes() {
  const scratch = new DataView(new ArrayBuffer(4096));
  return encodeState(scratch, PROBE_STATE);
}
