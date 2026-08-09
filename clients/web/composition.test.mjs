import assert from "node:assert/strict";

import { buildInstrumentStages, enumerateComposition } from "./composition.js";
import { REASON } from "./instrument-health.js";

function fakeEnumeration() {
  const panels = ["pfd", "hsi", "monitor"];
  const titles = ["PFD", "HSI", "Monitor"];
  return {
    composition_slot_count: () => panels.length,
    composition_slot_panel: (index) => panels[index] ?? "",
    composition_slot_x: (index) => index * 480,
    composition_slot_y: () => 0,
    composition_slot_width: () => 480,
    composition_slot_height: () => 360,
    panel_count: () => panels.length,
    panel_id: (index) => panels[index] ?? "",
    panel_title: (index) => titles[index] ?? "",
  };
}

function fakeDocument() {
  class Element {
    constructor(tagName) {
      this.tagName = tagName;
      this.children = [];
      this.className = "";
      this.id = "";
      this.textContent = "";
      this.value = "";
      this.width = 0;
      this.height = 0;
    }

    append(...children) {
      this.children.push(...children);
    }
  }

  return {
    createElement: (tagName) => new Element(tagName),
    Element,
  };
}

const slots = enumerateComposition(fakeEnumeration());
assert.deepEqual(
  slots.map(({ panel, title }) => [panel, title]),
  [
    ["pfd", "PFD"],
    ["hsi", "HSI"],
    ["monitor", "Monitor"],
  ],
);

const savedDocument = globalThis.document;
const documentDouble = fakeDocument();
globalThis.document = documentDouble;
try {
  const column = new documentDouble.Element("section");
  const mainView = new documentDouble.Element("select");
  buildInstrumentStages(slots, { column, mainView });
  assert.equal(column.children.length, 3);
  assert.equal(mainView.children.length, 3);
  assert.equal(column.children[2].id, "stage-monitor");
  assert.equal(column.children[2].children[1].children[0].id, "monitor");
} finally {
  globalThis.document = savedDocument;
}

assert.throws(
  () =>
    enumerateComposition({
      ...fakeEnumeration(),
      composition_slot_width: () => 0,
    }),
  (error) => error?.reason === REASON.ABI_MISMATCH,
);

console.log("screen composition tests passed");
