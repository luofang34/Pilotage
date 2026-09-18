// Complete before-paint identity gate for the instrument runtime.

import { InstrumentFault, REASON } from "./instrument-health.js";
import { STATE_ABI_VERSION } from "./state-abi.js";

export const EXPECTED_SCENE_FORMAT_VERSION = 1;
export const EXPECTED_CORPUS_VERSION = 8;
export const EXPECTED_CORPUS_DIGEST =
  "0b0c7ccb135bfc4107bc110e4b24dceffd84adf1b767fcd14d2c5ace7391f962";
export const EXPECTED_SCENE_DIGEST =
  "f6fb603bf2e1f7889ddc9b5d534f329a0379d34a8f00d954f366e9b270ddc273";
export const EXPECTED_COMPOSITION_DIGEST =
  "2912562c50aa7f6dd4bfd3b3be2c83fe69ab0ef225b6081496b5a0a5dd8f18f4";

export const COMPATIBILITY_BINDING_FNS = [
  "scene_format_version",
  "corpus_version",
  "corpus_digest_hex",
  "scene_digest_hex",
  "composition_digest_hex",
];

export function verifyInstrumentCompatibility(queryAbiVersion, bindings) {
  const checks = [
    ["state ABI", STATE_ABI_VERSION, queryAbiVersion],
    ["scene format", EXPECTED_SCENE_FORMAT_VERSION, bindings.scene_format_version],
    ["corpus version", EXPECTED_CORPUS_VERSION, bindings.corpus_version],
    ["corpus digest", EXPECTED_CORPUS_DIGEST, bindings.corpus_digest_hex],
    ["registry scene digest", EXPECTED_SCENE_DIGEST, bindings.scene_digest_hex],
    ["screen composition digest", EXPECTED_COMPOSITION_DIGEST, bindings.composition_digest_hex],
  ];
  for (const [name, expected, query] of checks) {
    let actual;
    try {
      actual = query();
    } catch (error) {
      throw new InstrumentFault(REASON.ABI_MISMATCH, `instrument ${name} query failed: ${error}`);
    }
    if (actual !== expected) {
      throw new InstrumentFault(
        REASON.ABI_MISMATCH,
        `instrument ${name} mismatch: runtime=${actual} shell=${expected}`,
      );
    }
  }
}
