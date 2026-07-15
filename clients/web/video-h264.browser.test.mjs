// Boundary test for the H.264 decode path: a real Chromium decodes the
// recorded Annex-B fixture through the real viewer path (wasm classification
// + session machine + the WebCodecs platform adapter) and this driver asserts
// what the harness page observed — every fixture frame output at the recorded
// dimensions, every frame closed by the adapter, and visible pixels on the
// canvas. Node-side fakes cannot witness any of this; only a browser can.
//
// Fail closed: no usable Chromium (set CHROME to override discovery) is a
// test failure, not a skip — CI must actually decode the fixture.

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const webRoot = dirname(fileURLToPath(import.meta.url));
const fixturePath = join(
  webRoot,
  "../../crates/pilotage-protocol/tests/fixtures/h264-annexb-baseline.h264",
);
// The recorded fixture (see its .provenance.md): five 48x32 frames.
const FIXTURE_FRAMES = 5;
const FIXTURE_WIDTH = 48;
const FIXTURE_HEIGHT = 32;

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

const contentTypes = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".wasm": "application/wasm",
  ".h264": "application/octet-stream",
};

// Serves exactly the files the harness needs; anything else is 404 so a
// harness regression (a new unpinned dependency) fails loud here.
const served = {
  "/": join(webRoot, "video-h264.browser.harness.html"),
  "/video-h264.js": join(webRoot, "video-h264.js"),
  "/instrument-runtime.js": join(webRoot, "instrument-runtime.js"),
  "/instrument-runtime_bg.wasm": join(webRoot, "instrument-runtime_bg.wasm"),
  "/fixtures/h264-annexb-baseline.h264": fixturePath,
};

let resolveResult;
const result = new Promise((resolve) => {
  resolveResult = resolve;
});

const server = createServer((req, res) => {
  if (req.method === "POST" && req.url === "/result") {
    let body = "";
    req.on("data", (chunk) => {
      body += chunk;
    });
    req.on("end", () => {
      res.writeHead(204).end();
      resolveResult(JSON.parse(body));
    });
    return;
  }
  const path = served[req.url];
  if (!path) {
    res.writeHead(404).end();
    return;
  }
  const dot = path.lastIndexOf(".");
  res.writeHead(200, { "content-type": contentTypes[path.slice(dot)] });
  res.end(readFileSync(path));
});

await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}/`;

const profile = mkdtempSync(join(tmpdir(), "pilotage-h264-chrome-"));
const chrome = spawn(
  chromeBinary(),
  [
    "--headless=new",
    "--no-sandbox",
    "--disable-gpu",
    "--no-first-run",
    "--disable-extensions",
    "--mute-audio",
    `--user-data-dir=${profile}`,
    origin,
  ],
  // Chrome is a process tree (browser, renderers, crashpad); detach it into
  // its own process group so teardown can kill the whole tree at once.
  { stdio: "ignore", detached: true },
);

const timeout = setTimeout(() => {
  console.error("FAIL - harness reported nothing within 60s");
  chrome.kill("SIGKILL");
  process.exit(1);
}, 60_000);

const observed = await result;
clearTimeout(timeout);
// Kill the whole process group and wait for the launcher to exit; helper
// processes can keep writing to the profile for a moment after that, so
// removal retries — and a profile that still lingers is a warning, not a
// verdict: the decode assertions below are the test.
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

check("harness ran without throwing", !observed.error);
if (observed.error) {
  console.error(observed.error);
  process.exit(1);
}
check(
  `fixture segments into ${FIXTURE_FRAMES} access units`,
  observed.units === FIXTURE_FRAMES,
);
check(
  `decoder outputs all ${FIXTURE_FRAMES} fixture frames`,
  observed.frames.length === FIXTURE_FRAMES,
);
check(
  `every frame is ${FIXTURE_WIDTH}x${FIXTURE_HEIGHT}`,
  observed.frames.every(
    (frame) => frame.width === FIXTURE_WIDTH && frame.height === FIXTURE_HEIGHT,
  ),
);
check(
  "the adapter closes every frame it is handed",
  observed.frames.every((frame) => frame.closedByAdapter === true),
);
check(
  "the canvas takes the stream's dimensions",
  observed.canvas.width === FIXTURE_WIDTH && observed.canvas.height === FIXTURE_HEIGHT,
);
check("decoded pixels are painted (canvas is not blank)", observed.painted === true);
check("the session did not fail", observed.failed === false);
check("no failure was logged", (observed.logs ?? []).length === 0);

if (failures > 0) {
  console.error(`${failures} check(s) failed`);
  process.exit(1);
}
console.log("browser H.264 fixture decode passed");
