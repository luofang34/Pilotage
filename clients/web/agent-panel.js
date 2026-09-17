// The agent panel: what the agent module reports, drawn into the page. The
// panel reads a status and writes elements. It decides nothing.

const SHOWN_ENTRIES = 8;

/** Draws one agent status into the panel elements. */
export function renderAgentPanel(els, status, document) {
  if (!els.agentStatus) return;
  els.agentStatus.textContent = status.engaged
    ? `AGENT HAS CONTROL · ${status.phase}`
    : "agent off";
  els.agentStatus.dataset.engaged = String(status.engaged);
  els.agentEngage.textContent = status.engaged ? "Release agent" : "Engage agent";
  els.agentModel.textContent = status.adapter ?? `model gateway ${status.gateway}`;
  els.agentMessage.disabled = !status.engaged;
  els.agentSend.disabled = !status.engaged;
  for (const preset of els.agentPresets) preset.disabled = !status.engaged;
  const entries = status.history.slice(-SHOWN_ENTRIES).map((entry) => {
    const item = document.createElement("li");
    item.dataset.kind = entry.kind;
    item.textContent = entry.text;
    return item;
  });
  els.agentLog.replaceChildren(...entries);
}
