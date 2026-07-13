// Exact unsigned wire-range predicates and typed rejection reasons for
// browser-side identity validators (GEO-68). Every field decoded from the wire
// has one exact numeric type (unsigned for identities and times, signed i64 for
// a clock-mapping offset); these enforce that type and its range,
// fail-closed, with no clamping. A value that is negative, fractional, out of
// range, or of the wrong numeric kind (a Number where a BigInt is required, or
// a BigInt where a Number is required) is rejected — never coerced. `firstFault`
// pairs the rules with field names so a validator reports exactly which field
// and which rule refused a value (a typed reason, not a bare boolean).

/** Largest u8 wire value. */
export const U8_MAX = 0xff;
/** Largest u32 wire value. */
export const U32_MAX = 0xffff_ffff;
/** Largest u64 wire value. */
export const U64_MAX = 0xffff_ffff_ffff_ffffn;
/** Most negative i64 wire value. */
export const I64_MIN = -(1n << 63n);
/** Largest i64 wire value. */
export const I64_MAX = (1n << 63n) - 1n;

/** Canonical 128-bit incarnation spelling: exactly 32 lowercase hex digits. */
export const INCARNATION_HEX = /^[0-9a-f]{32}$/;

/** The specific rule a field violated, for a typed rejection reason. */
export const RULE = Object.freeze({
  WRONG_KIND: "wrong-numeric-kind",
  NEGATIVE: "negative",
  FRACTIONAL: "fractional",
  OUT_OF_RANGE: "out-of-range",
  MALFORMED: "malformed",
});

/**
 * A Number-typed unsigned wire integer within `[0, max]`. Rejects a BigInt
 * (a Number field must not be given a BigInt), a fractional or non-finite
 * value, a negative value, and anything above `max`.
 */
export function isUintInRange(value, max) {
  return Number.isInteger(value) && value >= 0 && value <= max;
}

/** A u8 wire field (e.g. a video-frame source id). */
export function isU8(value) {
  return isUintInRange(value, U8_MAX);
}

/** A u32 wire field (epoch, sequence, camera id, calibration id). */
export function isU32(value) {
  return isUintInRange(value, U32_MAX);
}

/**
 * A BigInt-typed unsigned wire integer within `[0n, max]`. Rejects a Number
 * (a u64 field must not be given a Number — that silently truncates past
 * 2^53), a negative BigInt, and anything above `max`.
 */
export function isBigUintInRange(value, max) {
  return typeof value === "bigint" && value >= 0n && value <= max;
}

/** A u64 wire field (source id, acquisition/capture time nanoseconds). */
export function isU64(value) {
  return isBigUintInRange(value, U64_MAX);
}

/**
 * A signed BigInt-typed i64 wire field (e.g. a clock-mapping offset, which may
 * be negative). Rejects a Number (a truncating kind for a 64-bit field) and any
 * BigInt outside `[I64_MIN, I64_MAX]`.
 */
export function isI64(value) {
  return typeof value === "bigint" && value >= I64_MIN && value <= I64_MAX;
}

/** A 128-bit incarnation field: a string of exactly 32 lowercase hex digits. */
export function isIncarnation(value) {
  return typeof value === "string" && INCARNATION_HEX.test(value);
}

/** The typed rule a value violates for its kind, or `null` when it is valid. */
export function fieldFault(kind, value) {
  switch (kind) {
    case "u8":
    case "u32": {
      const max = kind === "u8" ? U8_MAX : U32_MAX;
      if (typeof value !== "number") return RULE.WRONG_KIND;
      if (!Number.isInteger(value)) return RULE.FRACTIONAL;
      if (value < 0) return RULE.NEGATIVE;
      return value > max ? RULE.OUT_OF_RANGE : null;
    }
    case "u64": {
      if (typeof value !== "bigint") return RULE.WRONG_KIND;
      if (value < 0n) return RULE.NEGATIVE;
      return value > U64_MAX ? RULE.OUT_OF_RANGE : null;
    }
    case "i64": {
      if (typeof value !== "bigint") return RULE.WRONG_KIND;
      return value < I64_MIN || value > I64_MAX ? RULE.OUT_OF_RANGE : null;
    }
    case "incarnation":
      return isIncarnation(value) ? null : RULE.MALFORMED;
    default:
      return RULE.MALFORMED;
  }
}

/**
 * The first field to violate its wire type/range in `spec`, as a typed
 * `{ field, rule }` reason, or `null` when every field is valid. `spec` is an
 * array of `[fieldName, kind, value]`. Evaluation order is the spec order, so
 * the reported field is deterministic.
 */
export function firstFault(spec) {
  for (const [field, kind, value] of spec) {
    const rule = fieldFault(kind, value);
    if (rule !== null) return { field, rule };
  }
  return null;
}
