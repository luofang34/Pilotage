const ELEMENT_IDS = [
  "host", "port", "certHash", "joinCode", "connectBtn", "resumeBtn",
  "status", "overlay", "telemetry", "gamepad", "flightMode", "feelMode",
  "agentStatus", "agentEngage", "agentModel", "agentMessage", "agentSend", "agentLog",
];

/** Resolves the fixed viewer elements from a document. */
export function resolveViewerElements(document) {
  return {
    ...Object.fromEntries(ELEMENT_IDS.map((id) => [id, document.getElementById(id)])),
    agentPresets: [...document.querySelectorAll("#agentPresets button")],
  };
}
