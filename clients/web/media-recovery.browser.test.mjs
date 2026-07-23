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
  const binary = candidates.find(existsSync);
  if (binary) return binary;
  console.error(`FAIL - no Chromium found (searched ${candidates.join(", ")})`);
  process.exit(1);
}

let resolveResult;
const result = new Promise((resolve) => {
  resolveResult = resolve;
});
const server = createServer((req, res) => {
  if (req.method === "POST" && req.url === "/result") {
    let body = "";
    req.on("data", (chunk) => { body += chunk; });
    req.on("end", () => {
      res.writeHead(204).end();
      resolveResult(JSON.parse(body));
    });
    return;
  }
  const requestPath = req.url === "/" ? "/media-recovery.browser.harness.html" : req.url;
  const path = normalize(join(webRoot, requestPath));
  if (!path.startsWith(webRoot) || !existsSync(path)) {
    res.writeHead(404).end();
    return;
  }
  res.writeHead(200, {
    "content-type": path.endsWith(".html") ? "text/html" : "text/javascript",
  });
  res.end(readFileSync(path));
});

await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const profile = mkdtempSync(join(tmpdir(), "pilotage-media-recovery-chrome-"));
const chrome = spawn(chromeBinary(), [
  "--headless=new",
  "--no-sandbox",
  "--disable-gpu",
  "--no-first-run",
  "--disable-extensions",
  "--remote-debugging-port=0",
  `--user-data-dir=${profile}`,
  `http://127.0.0.1:${server.address().port}/`,
], { stdio: "ignore", detached: true });

const timeout = setTimeout(() => {
  console.error("FAIL - media recovery browser harness reported nothing within 60s");
  try { process.kill(-chrome.pid, "SIGKILL"); } catch { chrome.kill("SIGKILL"); }
  process.exit(1);
}, 60_000);
const observed = await result;
clearTimeout(timeout);
const exited = new Promise((resolve) => chrome.once("exit", resolve));
try { process.kill(-chrome.pid, "SIGKILL"); } catch { chrome.kill("SIGKILL"); }
await exited;
server.close();
rmSync(profile, { recursive: true, force: true });

if (observed.error) {
  console.error(observed.error);
  process.exit(1);
}
const checks = {
  "mid-read abandonment is observed": observed.abandoned,
  "reader cancellation carries its typed reason": observed.cancelKind === "stream-abandoned",
  "stream credit is available for replacement frames": observed.creditRecovered,
  "all-source stall requests media attachment": observed.requested,
  "request uses the typed media-attach envelope": observed.typedRequest,
  "recovery stays on the live transport session": observed.sameSession,
  "both video sources resume": observed.resumed,
  "recovery does not refresh the page": observed.navigationCount === 1,
};
let failures = 0;
for (const [name, ok] of Object.entries(checks)) {
  console[ok ? "log" : "error"](`${ok ? "ok" : "FAIL"} - ${name}`);
  if (!ok) failures += 1;
}
if (failures > 0) process.exit(1);
console.log("headless-CDP media recovery chain passed");
