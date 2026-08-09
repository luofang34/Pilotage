// Screen-composition client of the instrument wasm (ADR-0032). The
// cockpit's instrument stages, the layout's home map, and the health
// maps all derive from the one validated composition the runtime
// exports; the page holds no panel list of its own. Slot index is the
// paint order and the column order.
//
// Video, gimbal, and chase stay static HTML: they are not Indicate
// panels, so the composition does not reach them (ADR-0032).

import { InstrumentFault, REASON } from "./instrument-health.js";

let compositionPromise = null;

/** The memoized composition slots, or a rejection every caller must
 *  catch. A failed load leaves the page without instrument stages; the
 *  ordinary instrument load path then reports the same failure
 *  fail-visibly. */
export function loadCompositionSlots() {
  compositionPromise ??= (async () => {
    let bindings;
    try {
      bindings = await import("./instrument-runtime.js");
      await bindings.default();
    } catch (error) {
      throw new InstrumentFault(REASON.WASM_LOAD, `composition runtime load failed: ${error}`);
    }
    return enumerateComposition(bindings);
  })();
  return compositionPromise;
}

/** Reads the composition enumeration into frozen slot records, adding
 *  each slot panel's title from the panel enumeration. Throws on an
 *  invalid slot: a broken composition is a load failure, not a partial
 *  cockpit. */
export function enumerateComposition(enumeration) {
  const required = [
    "composition_slot_count",
    "composition_slot_panel",
    "composition_slot_x",
    "composition_slot_y",
    "composition_slot_width",
    "composition_slot_height",
    "panel_count",
    "panel_id",
    "panel_title",
  ];
  for (const name of required) {
    if (typeof enumeration?.[name] !== "function") {
      throw new InstrumentFault(REASON.ABI_MISMATCH, `composition binding lacks ${name}`);
    }
  }
  let count;
  try {
    count = enumeration.composition_slot_count();
  } catch (error) {
    throw new InstrumentFault(REASON.ABI_MISMATCH, `composition count query failed: ${error}`);
  }
  if (!Number.isInteger(count) || count < 1) {
    throw new InstrumentFault(REASON.ABI_MISMATCH, `composition slot count invalid: ${count}`);
  }
  const slots = [];
  for (let index = 0; index < count; index += 1) {
    const panel = enumeration.composition_slot_panel(index);
    const x = enumeration.composition_slot_x(index);
    const y = enumeration.composition_slot_y(index);
    const width = enumeration.composition_slot_width(index);
    const height = enumeration.composition_slot_height(index);
    if (typeof panel !== "string" || panel.length === 0 || !(width > 0) || !(height > 0)) {
      throw new InstrumentFault(REASON.ABI_MISMATCH, `composition slot ${index} invalid`);
    }
    slots.push(
      Object.freeze({
        index,
        panel,
        title: titleFor(enumeration, panel),
        x,
        y,
        width,
        height,
      }),
    );
  }
  return Object.freeze(slots);
}

function titleFor(enumeration, panelId) {
  const count = enumeration.panel_count();
  for (let index = 0; index < count; index += 1) {
    if (enumeration.panel_id(index) === panelId) return enumeration.panel_title(index);
  }
  return panelId;
}

/** Creates one instrument stage per slot, in slot order, into `column`:
 *  figure#stage-<panel> with a figcaption and a .frame canvas whose id
 *  is the panel id and whose size is the slot rect (the frame the panel
 *  emits at). Also appends a main-view option per stage. Canvas ids stay
 *  stable because slot panel ids are the contract. */
export function buildInstrumentStages(slots, { column, mainView }) {
  for (const slot of slots) {
    const figure = document.createElement("figure");
    figure.className = "stage";
    figure.id = `stage-${slot.panel}`;
    const caption = document.createElement("figcaption");
    caption.textContent = slot.title;
    const frame = document.createElement("div");
    frame.className = "frame";
    const canvas = document.createElement("canvas");
    canvas.id = slot.panel;
    canvas.width = Math.round(slot.width);
    canvas.height = Math.round(slot.height);
    frame.append(canvas);
    figure.append(caption, frame);
    column.append(figure);
    if (mainView) {
      const option = document.createElement("option");
      option.value = figure.id;
      option.textContent = slot.title;
      mainView.append(option);
    }
  }
}
