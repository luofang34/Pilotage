// State-frame writer for the record groups and the cockpit clock: the
// JS side of the presence-masked sample codec and the clock codec in
// the indicate-instrument-state crate (pinned upstream).
//
// A record group is a header (record count u16, reserved u16, age f32)
// and then one record per engine or tank. A record is a presence mask
// u16 and then one f32 for each set bit, in ascending bit order. The
// bit of a field is its index in the field list below. Each list is
// append-only upstream, so a field keeps its bit.

const TAG = Object.freeze({
  ENGINE: 0x0e,
  FUEL_TANKS: 0x18,
  CLOCK: 0x19,
});

const ENGINE_FIELDS = Object.freeze([
  "manifoldPressureHpa",
  "rpm",
  "oilTemperatureC",
  "oilPressureKpa",
  "fuelPressureKpa",
  "coolantTemperatureC",
  "cylinderHeadTemperatureC",
  "carburettorAirTemperatureC",
  "shaftAngleDeg",
]);

const TANK_FIELDS = Object.freeze(["quantityL", "capacityL", "quantityKg", "capacityKg"]);

const f = (view, off, v) => view.setFloat32(off, v ?? NaN, true);

// A field with no value is absent: its bit stays clear and no f32 is
// written. A null or undefined value is a field the source does not have.
function putRecord(view, off, record, fields) {
  let mask = 0;
  let at = off + 2;
  fields.forEach((name, bit) => {
    const value = record?.[name];
    if (value === undefined || value === null) return;
    mask |= 1 << bit;
    f(view, at, value);
    at += 4;
  });
  view.setUint16(off, mask, true);
  return at - off;
}

function recordGroup(fields) {
  return (view, off, group) => {
    const records = group.records ?? [];
    view.setUint16(off, records.length, true);
    view.setUint16(off + 2, 0, true);
    f(view, off + 4, group.ageMs);
    let len = 8;
    for (const record of records) {
      len += putRecord(view, off + len, record, fields);
    }
    return len;
  };
}

// Each entry is [tag, select, encode]. The encoder writes the group
// payload at `off` and returns its length.
export const RECORD_ENCODERS = [
  [TAG.ENGINE, (s) => s.engines, recordGroup(ENGINE_FIELDS)],
  [TAG.FUEL_TANKS, (s) => s.fuelTanks, recordGroup(TANK_FIELDS)],
  [
    TAG.CLOCK,
    (s) => s.clock,
    (view, off, clock) => {
      f(view, off, clock.timeOfDayS);
      f(view, off + 4, clock.ageMs);
      return 8;
    },
  ],
];

// One state fragment that holds each group of this module with no
// records. Records make a frame longer without limit, so the frame-size
// probe measures the groups, and a source with records sizes its own
// buffer.
export const RECORD_PROBE_STATE = Object.freeze({
  engines: { records: [], ageMs: 0 },
  fuelTanks: { records: [], ageMs: 0 },
  clock: { ageMs: 0 },
});
