/** Resolves the fixed viewer elements from a document. */
export function resolveViewerElements(document) {
  return {
    host: document.getElementById("host"),
    port: document.getElementById("port"),
    certHash: document.getElementById("certHash"),
    connectBtn: document.getElementById("connectBtn"),
    resumeBtn: document.getElementById("resumeBtn"),
    status: document.getElementById("status"),
    overlay: document.getElementById("overlay"),
    telemetry: document.getElementById("telemetry"),
    gamepad: document.getElementById("gamepad"),
    flightMode: document.getElementById("flightMode"),
  };
}
