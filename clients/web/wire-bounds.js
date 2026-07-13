// Exact unsigned wire-range predicates for browser-side identity validators
// (GEO-68). Every identity field decoded from the wire has one exact unsigned
// type; these predicates enforce that type and its range, fail-closed, with no
// clamping. A value that is negative, fractional, out of range, or of the wrong
// numeric kind (a Number where a BigInt is required, or a BigInt where a Number
// is required) is rejected — never coerced. Reason strings pair with the
// predicates so a validator can report exactly which field and which rule
// refused a value.

/** Largest u8 wire value. */
export const U8_MAX = 0xff;
/** Largest u32 wire value. */
export const U32_MAX = 0xffff_ffff;
/** Largest u64 wire value. */
export const U64_MAX = 0xffff_ffff_ffff_ffffn;

/** Canonical 128-bit incarnation spelling: exactly 32 lowercase hex digits. */
export const INCARNATION_HEX = /^[0-9a-f]{32}$/;

/**
 * A Number-typed unsigned wire integer within `[0, max]`. Rejects a BigInt
 * (a Number field must not be given a BigInt), a fractional or non-finite
 * value, a negative value, and anything above `max`. `Number.isInteger`
 * already excludes BigInt, NaN, Infinity, and fractions.
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

/** A 128-bit incarnation field: a string of exactly 32 lowercase hex digits. */
export function isIncarnation(value) {
  return typeof value === "string" && INCARNATION_HEX.test(value);
}
