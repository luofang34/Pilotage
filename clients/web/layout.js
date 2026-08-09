// Cockpit layout wiring: the main-view selector and the collapsible log.
// Pure page furniture, deliberately separate from main.js — it only moves
// figures between the main slot, the G5 column, and the hidden shelf; every
// canvas keeps its id, so the render paths never notice. Captions and the
// SIM banner live outside the boxes (index.html), so nothing here may
// overlay instrument or video pixels.
//
// The instrument stages are composition data (ADR-0032): this module
// builds them from the wasm composition slots, and slot order is the
// column order. Video and chase stay static HTML — they are not panels.

import { buildInstrumentStages } from "./composition.js";
import { loadCompositionForPage } from "./instrument-startup.js";

const mainSlot = document.getElementById("mainSlot");
const g5Column = document.getElementById("g5Column");
const shelf = document.getElementById("stageShelf");
const mainView = document.getElementById("mainView");

// The independent page-level failure surface does not require a slot.
const { slots } = await loadCompositionForPage();
buildInstrumentStages(slots, { column: g5Column, mainView });
const ORDER = slots.map((slot) => `stage-${slot.panel}`);

/** Where a figure belongs when it is NOT the main view. Instruments
 *  home to the G5 column from the composition; video entries stay
 *  static. */
const HOME = {
  "stage-video": shelf,
  "stage-chase": shelf,
  ...Object.fromEntries(ORDER.map((id) => [id, g5Column])),
};

/** Returns a figure to its home. The G5 column keeps slot order: a
 *  returning figure inserts before the first later slot. */
function returnHome(figure, home) {
  if (home !== g5Column) {
    home.append(figure);
    return;
  }
  const rank = ORDER.indexOf(figure.id);
  const next = [...g5Column.querySelectorAll("figure.stage")].find(
    (candidate) => ORDER.indexOf(candidate.id) > rank,
  );
  g5Column.insertBefore(figure, next ?? null);
}

/** Moves the selected figure into the main slot and sends the previous
 *  occupant back to its home container. */
function selectMainView(figureId) {
  const incoming = document.getElementById(figureId);
  if (!incoming) return;
  const outgoing = mainSlot.querySelector("figure.stage");
  if (outgoing === incoming) return;
  if (outgoing) returnHome(outgoing, HOME[outgoing.id] ?? shelf);
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
