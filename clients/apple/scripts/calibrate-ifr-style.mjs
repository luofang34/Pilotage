#!/usr/bin/env node
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const [source, profile, latitudeText, stylePath] = process.argv.slice(2);
const chartView = await import(pathToFileURL(resolve(source, "web/rust/chart-view.js")));
const { applyPaperScale } = await import(pathToFileURL(resolve(source, "web/rust/paper-scale.js")));
const latitude = Number(latitudeText);
const specification = chartView.PAPER_PROFILES[profile];
if (!specification) throw new Error(`Unknown chart profile: ${profile}`);
const zoom = chartView.paperZoom(latitude, specification.nmPerInch, 96);
const style = JSON.parse(await readFile(stylePath, "utf8"));
for (const layer of style.layers) applyPaperScale(layer, zoom);
style.metadata ??= {};
style.metadata["pilotage:paper-calibration"] = {
    latitude, reference_zoom: zoom, pixels_per_inch: 96,
    nautical_miles_per_inch: specification.nmPerInch,
    reference: specification.reference,
};
process.stdout.write(JSON.stringify(style));
