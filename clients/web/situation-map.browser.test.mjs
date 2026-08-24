// Boundary test for the situation map stage: a real Chromium loads the REAL
// entrypoint (index.html + main.js, unmodified), selects the map stage, and
// this driver asserts the stage contract unit tests cannot see — with
// exported assets the map must reach the globe through the shared style at
// the Apple client's camera, and without assets or without the vendored
// renderer the stage must state a typed reason instead of an empty map.
//
// The exported fixture assets come from the REAL export script run against
// tiny generated archives, so the MBTiles row flip, the vector-tile
// decompression, and the TileJSON emission are all under test here too.
//
// Fail closed: no usable Chromium (set CHROME to override discovery) is a
// test failure, not a skip.
//
// Run: node clients/web/situation-map.browser.test.mjs

import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const webRoot = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(webRoot, "..", "..");
const TEST_PORT_PARAM = "4433";
const TEST_CERT_PARAM = "a".repeat(64);

let failures = 0;
function check(name, ok) {
  if (ok) {
    console.log(`ok - ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL - ${name}`);
  }
}

function chromeBinary() {
  const candidates = [
    process.env.CHROME,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium-browser",
    "/usr/bin/chromium",
  ].filter(Boolean);
  for (const candidate of candidates) {
    if (existsSync(candidate)) return candidate;
  }
  console.error(
    `FAIL - no Chromium found (set CHROME to a binary; searched ${candidates.join(", ")})`,
  );
  process.exit(1);
}

// The vendored renderer is a build artifact; the boot contract needs the
// real files, so their absence fails the test rather than skipping it.
const vendorDir = join(webRoot, "vendor", "maplibre-gl");
if (!existsSync(join(vendorDir, "maplibre-gl.mjs"))) {
  console.error(
    "FAIL - the renderer is not vendored; run scripts/vendor-maplibre-web.sh first",
  );
  process.exit(1);
}

// Build the fixture assets once: two tiny archives beside the REAL shared
// style and terrain manifest, exported by the REAL export script.
const fixtureRoot = mkdtempSync(join(tmpdir(), "pilotage-map-assets-"));
const fixtureResources = join(fixtureRoot, "Resources");
const fixtureAssets = join(fixtureRoot, "situation-assets");
{
  const generate = spawnSync(
    "python3",
    ["-", fixtureResources, join(repoRoot, "clients", "apple", "Resources")],
    {
      input: `
import gzip
import shutil
import sqlite3
import struct
import sys
import zlib
from pathlib import Path

fixture = Path(sys.argv[1])
real = Path(sys.argv[2])
fixture.mkdir(parents=True)
shutil.copyfile(real / "SituationStyle.json", fixture / "SituationStyle.json")
shutil.copyfile(
    real / "SituationTerrain.manifest.json",
    fixture / "SituationTerrain.manifest.json",
)
shutil.copytree(real / "Fonts", fixture / "Fonts")


def chunk(tag, data):
    piece = struct.pack(">I", len(data)) + tag + data
    return piece + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)


# One 256x256 Terrarium PNG, every pixel at elevation zero (128, 0, 0).
size = 256
raw = b"".join(b"\\x00" + bytes([128, 0, 0]) * size for _ in range(size))
png = (
    b"\\x89PNG\\r\\n\\x1a\\n"
    + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0))
    + chunk(b"IDAT", zlib.compress(raw))
    + chunk(b"IEND", b"")
)


def archive(path, rows, fmt):
    connection = sqlite3.connect(path)
    connection.execute("CREATE TABLE metadata (name text, value text)")
    connection.execute(
        "CREATE TABLE tiles (zoom_level integer, tile_column integer,"
        " tile_row integer, tile_data blob)"
    )
    connection.executemany(
        "INSERT INTO metadata VALUES (?, ?)",
        [("name", "fixture"), ("format", fmt)],
    )
    connection.executemany("INSERT INTO tiles VALUES (?, ?, ?, ?)", rows)
    connection.commit()
    connection.close()


# An empty vector tile is a valid tile with no layers; stored gzipped
# exactly as the real archives store theirs. The second tile sits at a
# zoom where the TMS row flip is observable: row 0 at zoom 1 must export
# as y=1, never y=0.
empty = gzip.compress(b"")
archive(
    fixture / "SituationCoastline.mbtiles",
    [(0, 0, 0, empty), (1, 0, 0, empty)],
    "pbf",
)
archive(fixture / "SituationTerrain.mbtiles", [(0, 0, 0, png)], "png")
`,
      encoding: "utf8",
    },
  );
  if (generate.status !== 0) {
    console.error(`FAIL - fixture archive generation failed:\n${generate.stderr}`);
    process.exit(1);
  }
  const exported = spawnSync(
    "bash",
    [
      join(repoRoot, "scripts", "build-web-situation-assets.sh"),
      "--resources",
      fixtureResources,
      "--out",
      fixtureAssets,
    ],
    { encoding: "utf8" },
  );
  if (exported.status !== 0) {
    console.error(`FAIL - asset export failed:\n${exported.stderr}`);
    process.exit(1);
  }
}

// The export contract, pinned on the files themselves: the TMS row flip
// and the vector-tile decompression.
check(
  "export: zoom-1 row 0 lands at y=1 (TMS row flip)",
  existsSync(join(fixtureAssets, "coastline", "1", "0", "1.pbf")) &&
    !existsSync(join(fixtureAssets, "coastline", "1", "0", "0.pbf")),
);
{
  const tile = readFileSync(join(fixtureAssets, "coastline", "0", "0", "0.pbf"));
  check(
    "export: vector tiles are stored decompressed",
    !(tile.length >= 2 && tile[0] === 0x1f && tile[1] === 0x8b),
  );
}

const contentTypes = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".css": "text/css",
  ".pbf": "application/x-protobuf",
  ".png": "image/png",
};

const errorCapturePrelude = `<script>
window.__bootErrors = [];
addEventListener("error", (e) => window.__bootErrors.push(String(e.error ?? e.message)));
addEventListener("unhandledrejection", (e) => window.__bootErrors.push("unhandledrejection: " + String(e.reason)));
</script>`;

// Runs after the entrypoint module graph: waits for boot to settle, selects
// the map stage, then waits for the stage to reach ready or a typed
// unavailable state and reports what the page actually shows.
const resultProbe = `<script type="module">
let deadline = Date.now() + 40000;
const statusText = () => document.getElementById("status")?.textContent ?? "";
const bootSettled = () =>
  statusText().includes("instrument panels ready") ||
  statusText().includes("instrument panels unavailable");
while (!bootSettled() && Date.now() < deadline) {
  await new Promise((resolve) => setTimeout(resolve, 100));
}
// The map wait gets its own budget; a slow viewer boot must not eat it.
deadline = Date.now() + 40000;
const select = document.getElementById("mainView");
const figure = document.getElementById("stage-map");
let optionPresent = false;
if (select && figure) {
  optionPresent = Boolean(select.querySelector('option[value="stage-map"]'));
  select.value = "stage-map";
  select.dispatchEvent(new Event("change", { bubbles: true }));
}
const mapSettled = () =>
  figure?.dataset.mapState === "ready" || figure?.dataset.mapState === "unavailable";
while (!mapSettled() && Date.now() < deadline) {
  await new Promise((resolve) => setTimeout(resolve, 100));
}
const surface = document.getElementById("situationMap");
const attribution = surface?.querySelector(".maplibregl-ctrl-attrib")?.textContent ?? "";
const notice = figure?.querySelector(".map-notice")?.textContent ?? "";
const inMainSlot = Boolean(figure?.closest("#mainSlot"));
const surfaceBox = surface?.getBoundingClientRect() ?? { width: 0, height: 0 };
await fetch("/map-result", {
  method: "POST",
  body: JSON.stringify({
    bootErrors: window.__bootErrors,
    optionPresent,
    inMainSlot,
    surfaceSize: { width: surfaceBox.width, height: surfaceBox.height },
    mapState: figure?.dataset.mapState ?? null,
    mapReason: figure?.dataset.mapReason ?? null,
    projection: figure?.dataset.mapProjection ?? null,
    zoom: figure?.dataset.mapZoom ?? null,
    pitch: figure?.dataset.mapPitch ?? null,
    center: figure?.dataset.mapCenter ?? null,
    maxZoom: figure?.dataset.mapMaxZoom ?? null,
    attribution,
    notice,
    videoCanvasUsable: Boolean(document.getElementById("video")?.getContext("2d")),
  }),
});
</script>`;

const ENTRYPOINT_TAG = '<script type="module" src="./main.js"></script>';

function instrumentedIndex() {
  const raw = readFileSync(join(webRoot, "index.html"), "utf8");
  if (!raw.includes(ENTRYPOINT_TAG)) {
    console.error(`FAIL - index.html no longer contains ${ENTRYPOINT_TAG}; update this test's anchor`);
    process.exit(1);
  }
  return raw.replace(ENTRYPOINT_TAG, `${errorCapturePrelude}\n${ENTRYPOINT_TAG}\n${resultProbe}`);
}

/** Boots the real viewer, selects the map stage, and reports the probe's
 *  observation. `serveAssets` / `serveVendor` answer those trees with 404
 *  to drive the typed unavailable paths. */
async function bootScenario({ label, serveAssets, serveVendor }) {
  let resolveResult;
  const result = new Promise((resolve) => {
    resolveResult = resolve;
  });

  const page = instrumentedIndex();
  const server = createServer((req, res) => {
    if (req.method === "POST" && req.url === "/map-result") {
      let body = "";
      req.on("data", (piece) => {
        body += piece;
      });
      req.on("end", () => {
        res.writeHead(204).end();
        resolveResult(JSON.parse(body));
      });
      return;
    }
    const path = (req.url ?? "/").split("?")[0];
    if (path === "/" || path === "/index.html") {
      res.writeHead(200, { "content-type": "text/html" });
      res.end(page);
      return;
    }
    // The exported assets live outside the web tree in this test; serve the
    // fixture export where the client expects /situation-assets/.
    let resolved;
    if (path.startsWith("/situation-assets/")) {
      if (!serveAssets) {
        res.writeHead(404).end();
        return;
      }
      resolved = normalize(join(fixtureAssets, path.slice("/situation-assets/".length)));
      if (!resolved.startsWith(fixtureAssets)) {
        res.writeHead(404).end();
        return;
      }
    } else {
      if (!serveVendor && path.startsWith("/vendor/")) {
        res.writeHead(404).end();
        return;
      }
      resolved = normalize(join(webRoot, path));
      if (!resolved.startsWith(webRoot)) {
        res.writeHead(404).end();
        return;
      }
    }
    const dot = resolved.lastIndexOf(".");
    const contentType = dot >= 0 ? contentTypes[resolved.slice(dot)] : undefined;
    if (!contentType || !existsSync(resolved)) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, { "content-type": contentType });
    res.end(readFileSync(resolved));
  });

  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const url = `${origin}/index.html?host=127.0.0.1&port=${TEST_PORT_PARAM}&cert=${TEST_CERT_PARAM}`;

  const profile = mkdtempSync(join(tmpdir(), "pilotage-map-chrome-"));
  const chrome = spawn(
    chromeBinary(),
    [
      "--headless=new",
      "--no-sandbox",
      "--no-first-run",
      "--disable-extensions",
      "--mute-audio",
      // The renderer needs WebGL2; newer Chromium builds refuse the
      // software fallback without this opt-in.
      "--enable-unsafe-swiftshader",
      `--user-data-dir=${profile}`,
      "--window-size=1280,900",
      url,
    ],
    { stdio: "ignore", detached: true },
  );

  const timeout = setTimeout(() => {
    console.error(`FAIL - ${label}: probe reported nothing within 90s`);
    try {
      process.kill(-chrome.pid, "SIGKILL");
    } catch {
      chrome.kill("SIGKILL");
    }
    process.exit(1);
  }, 90_000);

  const observed = await result;
  clearTimeout(timeout);
  const exited = new Promise((resolve) => chrome.once("exit", resolve));
  try {
    process.kill(-chrome.pid, "SIGKILL");
  } catch {
    chrome.kill("SIGKILL");
  }
  await exited;
  server.close();
  for (let attempt = 0; ; attempt += 1) {
    try {
      rmSync(profile, { recursive: true, force: true });
      break;
    } catch (error) {
      if (attempt >= 20) {
        console.warn(`leaving temp profile ${profile}: ${error}`);
        break;
      }
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
  return observed;
}

const near = (value, expected, tolerance) =>
  Math.abs(Number(value) - expected) <= tolerance;

// Scenario 1: assets and renderer present. The stage must reach the globe
// with the shared style at the Apple client's camera, and the source
// notices must be on screen.
{
  const observed = await bootScenario({
    label: "map ready",
    serveAssets: true,
    serveVendor: true,
  });
  check("map ready: no uncaught boot errors", observed.bootErrors.length === 0);
  if (observed.bootErrors.length > 0) console.error(observed.bootErrors.join("\n"));
  check("map ready: the main-view select offers the map stage", observed.optionPresent);
  check("map ready: selecting moves the stage into the main slot", observed.inMainSlot);
  check(
    "map ready: the map surface has real size",
    observed.surfaceSize.width > 200 && observed.surfaceSize.height > 200,
  );
  check("map ready: the stage reports ready", observed.mapState === "ready");
  check(`map ready: the projection is the globe (${observed.projection})`, observed.projection === "globe");
  check(`map ready: zoom matches the Apple camera (${observed.zoom})`, near(observed.zoom, 6, 0.01));
  check(`map ready: pitch matches the Apple camera (${observed.pitch})`, near(observed.pitch, 55, 0.01));
  const [lat, lng] = String(observed.center).split(",").map(Number);
  check(
    `map ready: centre matches the Apple camera (${observed.center})`,
    near(lat, 40.5, 0.01) && near(lng, -76.5, 0.01),
  );
  check(
    `map ready: closest zoom derives from the terrain manifest (${observed.maxZoom})`,
    near(observed.maxZoom, 15, 0.01),
  );
  check(
    "map ready: both source notices reach the screen",
    observed.attribution.includes("Natural Earth") &&
      observed.attribution.includes("AWS Terrain Tiles"),
  );
}

// Scenario 2: the assets are not exported. The stage must state the typed
// reason and the rest of the viewer must stay alive.
{
  const observed = await bootScenario({
    label: "assets missing",
    serveAssets: false,
    serveVendor: true,
  });
  check("assets missing: no uncaught boot errors", observed.bootErrors.length === 0);
  if (observed.bootErrors.length > 0) console.error(observed.bootErrors.join("\n"));
  check(
    "assets missing: the stage reports unavailable",
    observed.mapState === "unavailable",
  );
  check(
    "assets missing: the reason is MAP_ASSETS_MISSING",
    observed.mapReason === "MAP_ASSETS_MISSING",
  );
  check(
    "assets missing: the notice names the export script",
    observed.notice.includes("build-web-situation-assets.sh"),
  );
  check("assets missing: the video canvas stays usable", observed.videoCanvasUsable);
}

// Scenario 3: the renderer is not vendored. Same contract, its own reason.
{
  const observed = await bootScenario({
    label: "vendor missing",
    serveAssets: true,
    serveVendor: false,
  });
  check("vendor missing: no uncaught boot errors", observed.bootErrors.length === 0);
  if (observed.bootErrors.length > 0) console.error(observed.bootErrors.join("\n"));
  check(
    "vendor missing: the stage reports unavailable",
    observed.mapState === "unavailable",
  );
  check(
    "vendor missing: the reason is MAP_LIBRARY_MISSING",
    observed.mapReason === "MAP_LIBRARY_MISSING",
  );
  check(
    "vendor missing: the notice names the vendor script",
    observed.notice.includes("vendor-maplibre-web.sh"),
  );
}

rmSync(fixtureRoot, { recursive: true, force: true });

if (failures > 0) {
  console.error(`${failures} check(s) failed`);
  process.exit(1);
}
console.log("situation map stage contract passed");
