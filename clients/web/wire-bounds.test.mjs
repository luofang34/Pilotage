// Exhaustive tests for the exact unsigned wire-range predicates (GEO-68):
// every width rejects negative, fractional, over-range, and wrong-numeric-kind
// values, and never clamps.

import {
  INCARNATION_HEX,
  U32_MAX,
  U64_MAX,
  U8_MAX,
  isIncarnation,
  isU32,
  isU64,
  isU8,
  isUintInRange,
} from "./wire-bounds.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL - ${name}`);
  }
}

// ---- u8 ---------------------------------------------------------------------

check("u8 accepts 0 and the max", isU8(0) && isU8(U8_MAX));
check("u8 rejects one past the max", !isU8(U8_MAX + 1));
check("u8 rejects a negative", !isU8(-1));
check("u8 rejects a fraction", !isU8(1.5));
check("u8 rejects a bigint (wrong numeric kind)", !isU8(1n));
check("u8 rejects NaN and Infinity", !isU8(Number.NaN) && !isU8(Number.POSITIVE_INFINITY));

// ---- u32 --------------------------------------------------------------------

check("u32 accepts 0 and the max", isU32(0) && isU32(U32_MAX));
check("u32 rejects one past the max", !isU32(U32_MAX + 1));
check("u32 rejects a negative", !isU32(-1));
check("u32 rejects a fraction", !isU32(3.14));
check("u32 rejects a bigint", !isU32(5n));

// ---- u64 --------------------------------------------------------------------

check("u64 accepts 0n and the max", isU64(0n) && isU64(U64_MAX));
check("u64 rejects one past the max", !isU64(U64_MAX + 1n));
check("u64 rejects a negative bigint", !isU64(-1n));
check(
  "u64 rejects a Number (would truncate past 2^53)",
  !isU64(1000) && !isU64(0) && !isU64(Number.MAX_SAFE_INTEGER),
);
check("u64 rejects a fractional Number too", !isU64(1.5));

// ---- incarnation ------------------------------------------------------------

check("incarnation accepts 32 lowercase hex", isIncarnation("0123456789abcdef0123456789abcdef"));
check("incarnation rejects wrong length", !isIncarnation("abc"));
check("incarnation rejects uppercase", !isIncarnation("0123456789ABCDEF0123456789ABCDEF"));
check("incarnation rejects a non-string", !isIncarnation(0x123n) && !isIncarnation(null));
check("the incarnation regex is anchored", INCARNATION_HEX.source.startsWith("^"));

// ---- generic range ----------------------------------------------------------

check("isUintInRange respects an arbitrary max", isUintInRange(5, 5) && !isUintInRange(6, 5));

if (failures > 0) {
  console.error(`${failures} check(s) failed`);
  process.exit(1);
}
console.log("all wire-bounds checks passed");
