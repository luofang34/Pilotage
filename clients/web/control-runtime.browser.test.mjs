// Browser integration: a real Chromium loads the control module the way the
// viewer does — import control-shell.js, fetch the control-runtime wasm,
// bootstrap through the default profile, and execute one real control tick —
// then reports the plan. Direct helper tests run in node; only a browser
// proves the module graph, the wasm instantiation, and the typed-array memory
// views work together in the actual runtime environment.
//
// Fail closed: no usable Chromium (set CHROME to override) is a failure, not a
// skip. Build the wasm first: scripts/build-web-instruments.sh

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const webRoot = dirname(fileURLToPath(import.meta.url));

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
  console.error(`FAIL - no Chromium found (set CHROME; searched ${candidates.join(", ")})`);
  process.exit(1);
}

const contentTypes = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".wasm": "application/wasm",
};

const PAGE = `<!doctype html><meta charset="utf-8"><title>control boot</title>
<script type="module">
window.__errors = [];
addEventListener("error", (e) => window.__errors.push(String(e.error ?? e.message)));
addEventListener("unhandledrejection", (e) => window.__errors.push("rejection: " + String(e.reason)));
import { loadControlShell } from "./control-shell.js";
(async () => {
  try {
    const shell = await loadControlShell("./control-runtime_bg.wasm");
    // LT (button 6) held + right stick full: expect a gimbal frame with yaw=+1.
    const pad = {
      axes: [0, 0, 1, 1],
      buttons: Array.from({ length: 16 }, (_, i) => ({ pressed: i === 6, value: i === 6 ? 1 : 0 })),
    };
    const plan = shell.tickFromPad(pad, {
      mode: "quad-pilot", connected: true, leaseGranted: true, leaseDenied: false, nowMs: performance.now(),
    });
    await fetch("/result", { method: "POST", body: JSON.stringify({
      ran: true,
      revision: shell.activationRevision(),
      gimbalYaw: plan.gimbal ? plan.gimbal.yaw : null,
      motionRollMasked: plan.motion ? plan.motion.roll : null,
      errors: window.__errors,
    }) });
  } catch (error) {
    await fetch("/result", { method: "POST", body: JSON.stringify({ ran: false, error: String(error), errors: window.__errors }) });
  }
})();
</script>`;

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
  const path = (req.url ?? "/").split("?")[0];
  if (path === "/" || path === "/index.html") {
    res.writeHead(200, { "content-type": "text/html" });
    res.end(PAGE);
    return;
  }
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
const profile = mkdtempSync(join(tmpdir(), "pilotage-control-chrome-"));
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
    `${origin}/index.html`,
  ],
  { stdio: "ignore", detached: true },
);

const timeout = setTimeout(() => {
  console.error("FAIL - the control module reported nothing within 60s");
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
rmSync(profile, { recursive: true, force: true });

let failures = 0;
function check(name, ok) {
  if (ok) {
    console.log(`ok   - ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL - ${name}`);
  }
}

check("no uncaught errors during control boot", (observed.errors ?? []).length === 0);
if ((observed.errors ?? []).length > 0) console.error(observed.errors.join("\n"));
check("the control module booted and ran a tick", observed.ran === true);
check("the default profile activated (revision 1)", observed.revision === 1);
check("the tick produced a gimbal frame (LT captured the right stick)", observed.gimbalYaw === 1);
check("flight saw the captured right stick as neutral", observed.motionRollMasked === 0);

if (failures > 0) {
  console.error(`${failures} check(s) failed`);
  process.exit(1);
}
console.log("control module browser boot passed");
