// Failure-visible loading for the screen composition.

import { loadCompositionSlots } from "./composition.js";
import { REASON } from "./instrument-health.js";

export async function loadCompositionForPage({
  load = loadCompositionSlots,
  showFailure = showInstrumentStartupFailure,
} = {}) {
  try {
    return { slots: await load(), fault: null };
  } catch (error) {
    const fault = error?.reason ?? REASON.ABI_MISMATCH;
    try {
      showFailure(fault);
    } catch {
      // The page must continue to expose its non-instrument controls.
    }
    return { slots: Object.freeze([]), fault };
  }
}

// This surface does not depend on a composition slot or a loaded runtime.
export function showInstrumentStartupFailure(reason, doc = globalThis.document) {
  const id = "instrumentStartupFailure";
  let element = doc?.getElementById?.(id) ?? null;
  if (!element) {
    element = doc?.createElement?.("div") ?? null;
    if (!element) return false;
    element.id = id;
    element.setAttribute("role", "alert");
    element.setAttribute("aria-live", "assertive");
    Object.assign(element.style, {
      position: "fixed",
      inset: "1rem",
      zIndex: "1000",
      display: "grid",
      placeContent: "center",
      border: "6px solid #f00",
      background: "#000",
      color: "#f00",
      font: "bold 28px system-ui, sans-serif",
      textAlign: "center",
      whiteSpace: "pre-line",
    });
    doc?.body?.append?.(element);
  }
  element.textContent = `DISPLAY FAIL\nD-${reason}\nSIM / NOT FOR FLIGHT`;
  element.style.display = "grid";
  return true;
}
