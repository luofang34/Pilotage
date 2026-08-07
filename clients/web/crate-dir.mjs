// Resolves the on-disk directory of a Rust dependency crate via
// `cargo metadata`, so suites that read crate-owned artifacts (the
// scene-conformance corpus, the state-ABI golden frames) reach into the
// crate tree wherever cargo put it — for a pinned git dependency that is
// cargo's checkout of the exact pinned rev, which is what makes the
// upstream pin the single source of the bytes these suites verify.

import { execFileSync } from "node:child_process";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";

let packagesCache = null;

function packages() {
  if (packagesCache === null) {
    const raw = execFileSync(
      "cargo",
      ["metadata", "--format-version", "1", "--locked"],
      {
        cwd: dirname(dirname(dirname(fileURLToPath(import.meta.url)))),
        maxBuffer: 256 * 1024 * 1024,
        encoding: "utf8",
      },
    );
    packagesCache = JSON.parse(raw).packages;
  }
  return packagesCache;
}

// The absolute directory containing `name`'s Cargo.toml. Throws when the
// dependency graph does not contain exactly one package of that name:
// zero means the pin (or the workspace) lost the crate, two means the
// test graph split into duplicate identities — both are defects to
// surface, never to guess around.
export function crateDir(name) {
  const hits = packages().filter((p) => p.name === name);
  if (hits.length !== 1) {
    throw new Error(
      `expected exactly one package named ${name} in cargo metadata, found ${hits.length}`,
    );
  }
  return dirname(hits[0].manifest_path);
}
