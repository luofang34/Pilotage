// Boundary test for viewer startup: a real Chromium loads the REAL
// entrypoint (index.html + main.js, unmodified) and this driver asserts the
// boot contract that unit tests cannot see — module evaluation must complete
// (no wasm-backed object may be constructed before the instrument wasm
// initializes), a successful wasm load must end in live panels, and a failed
// wasm load must degrade visibly without killing the rest of the viewer.
// The server injects only observation scaffolding around the untouched page:
// an error-capture prelude ahead of the entrypoint and a result-reporting
// probe behind it.
//
// Fail closed: no usable Chromium (set CHROME to override discovery) is a
// test failure, not a skip — CI must actually boot the viewer.

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const webRoot = dirname(fileURLToPath(import.meta.url));
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

const contentTypes = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".wasm": "application/wasm",
  ".json": "application/json",
};

// Runs before the entrypoint module: uncaught exceptions and unhandled
// rejections during boot are the exact failure class this test exists for.
const errorCapturePrelude = `<script>
window.__bootErrors = [];
addEventListener("error", (e) => window.__bootErrors.push(String(e.error ?? e.message)));
addEventListener("unhandledrejection", (e) => window.__bootErrors.push("unhandledrejection: " + String(e.reason)));
</script>`;

// Runs after the entrypoint module graph: waits for the instrument boot to
// settle either way, then reports what the page actually reached.
const resultProbe = `<script type="module">
const deadline = Date.now() + 20000;
const statusText = () => document.getElementById("status")?.textContent ?? "";
const settled = () =>
  statusText().includes("instrument panels ready") ||
  statusText().includes("instrument panels unavailable");
while (!settled() && Date.now() < deadline) {
  await new Promise((resolve) => setTimeout(resolve, 100));
}
let canvasUsable = false;
try {
  canvasUsable = Boolean(document.getElementById("video")?.getContext("2d"));
} catch {}
// Layout geometry (DISP-05 / #171). The DOM fault presenter is an
// inset:0 absolute element inserted into the instrument frame; an
// absolute element is sized/positioned by its nearest POSITIONED
// ancestor, so the frame carrying position:relative is exactly the
// invariant that scopes the presenter to that frame rather than the
// viewport. The session log must also stay in normal flow so expanding
// it never overlays instrument pixels.
let framePositioned = false;
let presenterIsFrameChild = false;
let logInFlow = false;
try {
  const frame = document.getElementById("pfd")?.parentElement;
  const presenter = frame?.querySelector('div[role="alert"]');
  framePositioned = frame ? getComputedStyle(frame).position === "relative" : false;
  presenterIsFrameChild = Boolean(presenter && presenter.parentElement === frame);
  const dock = document.querySelector(".log-dock");
  logInFlow = dock ? getComputedStyle(dock).position === "static" : false;
} catch {}
await fetch("/boot-result", {
  method: "POST",
  body: JSON.stringify({
    bootErrors: window.__bootErrors,
    port: document.getElementById("port")?.value ?? null,
    cert: document.getElementById("certHash")?.value ?? null,
    ready: statusText().includes("instrument panels ready (wasm loaded)"),
    unavailable: statusText().includes("instrument panels unavailable"),
    canvasUsable,
    framePositioned,
    presenterIsFrameChild,
    logInFlow,
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
  return raw.replace(
    ENTRYPOINT_TAG,
    `${errorCapturePrelude}\n${ENTRYPOINT_TAG}\n${resultProbe}`,
  );
}

/** Boots the real viewer against a static server and reports the probe's
 *  observation. `serveWasm: false` answers the wasm fetch with 404 to drive
 *  the wasm-load-failure path. */
async function bootScenario({ label, serveWasm }) {
  let resolveResult;
  const result = new Promise((resolve) => {
    resolveResult = resolve;
  });

  const page = instrumentedIndex();
  const server = createServer((req, res) => {
    if (req.method === "POST" && req.url === "/boot-result") {
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
    const path = (req.url ?? "/").split("?")[0];
    if (path === "/" || path === "/index.html") {
      res.writeHead(200, { "content-type": "text/html" });
      res.end(page);
      return;
    }
    if (!serveWasm && path === "/instrument-runtime_bg.wasm") {
      res.writeHead(404).end();
      return;
    }
    // Serve the real clients/web tree, jailed to webRoot; anything the page
    // asks for beyond it (favicon and the like) is a plain 404.
    const resolved = normalize(join(webRoot, path));
    const dot = resolved.lastIndexOf(".");
    const contentType = dot >= 0 ? contentTypes[resolved.slice(dot)] : undefined;
    if (!resolved.startsWith(webRoot) || !contentType || !existsSync(resolved)) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, { "content-type": contentType });
    res.end(readFileSync(resolved));
  });

  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const url = `${origin}/index.html?host=127.0.0.1&port=${TEST_PORT_PARAM}&cert=${TEST_CERT_PARAM}`;

  const profile = mkdtempSync(join(tmpdir(), "pilotage-boot-chrome-"));
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
      url,
    ],
    // Chrome is a process tree (browser, renderers, crashpad); detach it into
    // its own process group so teardown can kill the whole tree at once.
    { stdio: "ignore", detached: true },
  );

  const timeout = setTimeout(() => {
    console.error(`FAIL - ${label}: probe reported nothing within 60s`);
    try {
      process.kill(-chrome.pid, "SIGKILL");
    } catch {
      chrome.kill("SIGKILL");
    }
    process.exit(1);
  }, 60_000);

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

// Scenario 1: normal boot. Module evaluation must complete (URL params
// applied proves everything after the H.264 registry site ran) with zero
// uncaught errors, and the wasm init path must end in live panels — which
// also proves the registry construction that follows wasm load succeeded.
{
  const observed = await bootScenario({ label: "normal boot", serveWasm: true });
  check("normal boot: no uncaught boot errors", observed.bootErrors.length === 0);
  if (observed.bootErrors.length > 0) console.error(observed.bootErrors.join("\n"));
  check(
    "normal boot: module evaluation completed (URL params applied)",
    observed.port === TEST_PORT_PARAM && observed.cert === TEST_CERT_PARAM,
  );
  check("normal boot: instrument panels ready (wasm loaded)", observed.ready === true);
  // Layout geometry regression guards (DISP-05 / #171).
  check(
    "layout: instrument frame is positioned so the fault presenter anchors to it",
    observed.framePositioned === true,
  );
  check(
    "layout: the fault presenter is a child of the positioned frame (scoped to it, not the viewport)",
    observed.framePositioned === true && observed.presenterIsFrameChild === true,
  );
  check(
    "layout: the session log stays in normal flow (never overlays instrument pixels)",
    observed.logInFlow === true,
  );
}

// Scenario 2: the wasm fetch fails. The viewer must degrade visibly — panels
// report unavailable — while the rest of the page stays alive: module fully
// evaluated, MJPEG paint surface still usable. No uncaught errors allowed;
// the failure must travel the fail-visible path, not the exception path.
{
  const observed = await bootScenario({ label: "wasm-load failure", serveWasm: false });
  check("wasm failure: no uncaught boot errors", observed.bootErrors.length === 0);
  if (observed.bootErrors.length > 0) console.error(observed.bootErrors.join("\n"));
  check(
    "wasm failure: module evaluation completed (URL params applied)",
    observed.port === TEST_PORT_PARAM && observed.cert === TEST_CERT_PARAM,
  );
  check("wasm failure: panels report unavailable", observed.unavailable === true);
  check("wasm failure: panels did not claim ready", observed.ready !== true);
  check("wasm failure: MJPEG canvas still usable", observed.canvasUsable === true);
}

if (failures > 0) {
  console.error(`${failures} check(s) failed`);
  process.exit(1);
}
console.log("viewer boot contract passed");
