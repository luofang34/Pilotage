// Cockpit layout wiring: the main-view selector and the collapsible log.
// Pure page furniture, deliberately separate from main.js — it only moves
// figures between the main slot, the G5 column, and the hidden shelf; every
// canvas keeps its id, so the render paths never notice. Captions and the
// SIM banner live outside the boxes (index.html), so nothing here may
// overlay instrument or video pixels.

const mainSlot = document.getElementById("mainSlot");
const g5Column = document.getElementById("g5Column");
const shelf = document.getElementById("stageShelf");
const mainView = document.getElementById("mainView");

/** Where a figure belongs when it is NOT the main view. */
const HOME = {
  "stage-video": shelf,
  "stage-chase": shelf,
  "stage-pfd": g5Column,
  "stage-hsi": g5Column,
};

/** Moves the selected figure into the main slot and sends the previous
 *  occupant back to its home container (G5 figures return to the top of
 *  the column so the PFD stays above the HSI). */
function selectMainView(figureId) {
  const incoming = document.getElementById(figureId);
  if (!incoming) return;
  const outgoing = mainSlot.querySelector("figure.stage");
  if (outgoing === incoming) return;
  if (outgoing) {
    const home = HOME[outgoing.id] ?? shelf;
    if (home === g5Column && outgoing.id === "stage-pfd") {
      home.prepend(outgoing);
    } else {
      home.append(outgoing);
    }
  }
  mainSlot.append(incoming);
}

if (mainView) {
  mainView.addEventListener("change", () => selectMainView(mainView.value));
  selectMainView(mainView.value);
}

// Collapsible session log: one (newest) line by default — log entries are
// prepended newest-first, so the collapsed view shows the latest event.
const logToggle = document.getElementById("logToggle");
const status = document.getElementById("status");
if (logToggle && status) {
  logToggle.addEventListener("click", () => {
    const expanded = status.classList.toggle("expanded");
    status.classList.toggle("collapsed", !expanded);
    logToggle.textContent = expanded ? "▾ log" : "▸ log";
  });
}
