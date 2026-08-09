// Complete before-paint identity gate for the instrument runtime.

import { InstrumentFault, REASON } from "./instrument-health.js";
import { STATE_ABI_VERSION } from "./state-abi.js";

export const EXPECTED_SCENE_FORMAT_VERSION = 1;
export const EXPECTED_CORPUS_VERSION = 4;
export const EXPECTED_CORPUS_DIGEST =
  "1fb8e6de2734ff7506843b05869f39d501f0926599636c6110a7e3b0c6e1625e";
export const EXPECTED_SCENE_DIGEST =
  "f82d905643b48822de25665761ad3e29daa334d937f18b1e98a3e215353cb704";
export const EXPECTED_COMPOSITION_DIGEST =
  "6761e8e1ed137e682530274c8f02353d2ab40e7142a36cd4321a6835323b463c";

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
