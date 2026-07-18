//! Canonical, human-reviewable serialization of the corpus plus its reference
//! outcomes into the shared golden JSON.
//!
//! The serializer is deterministic (fixed key order, integer-only numbers, no
//! maps) so the drift-guard test can compare the regenerated string byte for
//! byte against the checked-in golden. Review metadata lives in the header
//! constants below: changing the corpus forces a version/reason edit here and a
//! regeneration, and CI only ever compares — it never rewrites the file.

#![allow(clippy::expect_used, clippy::panic)]

use std::string::{String, ToString};
use std::vec::Vec;

use pilotage_instrument_scene::{
    MAX_LAYER_COMMANDS, MAX_SCENE_BYTES, MAX_STACK_DEPTH, MAX_TEXT_BYTES,
};
use sha2::{Digest, Sha256};

use super::corpus::CorpusEntry;
use super::outcomes::{Outcome, outcome_of};
use crate::{MAX_DIMENSION, MAX_POLYGON_VERTICES, WORST_CASE_FRAME_BYTES};

const SCHEMA_VERSION: u32 = 2;
const CORPUS_VERSION: u32 = 3;
const REVIEW_REASON: &str = "Add text-centered-multichar: a center-anchored multi-digit run at a \
readout-like size, pinning that both backends place centered readout values from the same nominal \
advance (DISP-02's width-aware readout fitting relies on that agreement).";
const REVIEW_APPROVED_BY: &str =
    "REN-04 owner; regenerated goldens require human review and are never rewritten by CI.";

/// SHA-256 over the concatenation of every entry's replay bytes, in order.
pub(super) fn corpus_sha256(entries: &[CorpusEntry]) -> String {
    let mut hasher = Sha256::new();
    for entry in entries {
        hasher.update(&entry.bytes);
    }
    hex_bytes(&hasher.finalize())
}

/// The full golden manifest as a UTF-8 JSON document ending in a newline.
pub(super) fn manifest_json(entries: &[CorpusEntry]) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("{".to_string());
    header_lines(&mut lines, entries);
    lines.push("  \"entries\": [".to_string());
    let blocks: Vec<String> = entries
        .iter()
        .map(|entry| entry_block(entry, &outcome_of(entry)))
        .collect();
    lines.push(blocks.join(",\n"));
    lines.push("  ]".to_string());
    lines.push("}".to_string());
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn header_lines(lines: &mut Vec<String>, entries: &[CorpusEntry]) {
    lines.push(std::format!("  \"schemaVersion\": {SCHEMA_VERSION},"));
    lines.push(std::format!("  \"corpusVersion\": {CORPUS_VERSION},"));
    lines.push(
        "  \"generatedBy\": \"pilotage-instrument-raster REN-04 conformance reference\","
            .to_string(),
    );
    lines.push("  \"simOnly\": \"Canvas/browser backend is SIM / NOT FOR FLIGHT\",".to_string());
    lines.push(
        "  \"canonicalization\": \"trace args are Q8.8 floor(v*256); non-finite as nan/inf/-inf\","
            .to_string(),
    );
    lines.push("  \"review\": {".to_string());
    lines.push(std::format!("    \"reason\": {},", json_str(REVIEW_REASON)));
    lines.push(std::format!(
        "    \"approvedBy\": {}",
        json_str(REVIEW_APPROVED_BY)
    ));
    lines.push("  },".to_string());
    budget_lines(lines);
    lines.push(std::format!(
        "  \"corpusSha256\": {},",
        json_str(&corpus_sha256(entries))
    ));
}

fn budget_lines(lines: &mut Vec<String>) {
    lines.push("  \"budgets\": {".to_string());
    lines.push("    \"layerCount\": 6,".to_string());
    lines.push(std::format!(
        "    \"maxLayerCommands\": {MAX_LAYER_COMMANDS},"
    ));
    lines.push(std::format!("    \"maxStackDepth\": {MAX_STACK_DEPTH},"));
    lines.push(std::format!("    \"maxSceneBytes\": {MAX_SCENE_BYTES},"));
    lines.push(std::format!("    \"maxTextBytes\": {MAX_TEXT_BYTES},"));
    lines.push(std::format!("    \"maxDimension\": {MAX_DIMENSION},"));
    lines.push(std::format!(
        "    \"maxPolygonVertices\": {MAX_POLYGON_VERTICES},"
    ));
    lines.push(std::format!(
        "    \"coordLimitPx\": {},",
        crate::fixed::COORD_LIMIT_PX as i64
    ));
    lines.push(std::format!(
        "    \"worstCaseFrameBytes\": {WORST_CASE_FRAME_BYTES}"
    ));
    lines.push("  },".to_string());
}

fn entry_block(entry: &CorpusEntry, o: &Outcome) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("    {".to_string());
    lines.push(std::format!("      \"name\": {},", json_str(entry.name)));
    lines.push(std::format!(
        "      \"category\": {},",
        json_str(entry.category)
    ));
    lines.push(std::format!("      \"notes\": {},", opt_str(entry.notes)));
    source_lines(&mut lines, entry);
    lines.push(std::format!("      \"framingValid\": {},", o.framing_valid));
    lines.push(std::format!(
        "      \"decode\": {{ \"ok\": {}, \"error\": {} }},",
        o.decode_ok,
        opt_owned(&o.decode_error)
    ));
    gate_line(&mut lines, o);
    lines.push(std::format!(
        "      \"render\": {{ \"ok\": {}, \"error\": {} }},",
        o.render_ok,
        opt_owned(&o.render_error)
    ));
    lines.push(std::format!(
        "      \"commandTrace\": {},",
        opt_str_array(&o.command_trace)
    ));
    lines.push(std::format!(
        "      \"interpreterRejects\": {},",
        opt_owned(&o.interpreter_rejects)
    ));
    lines.push(std::format!(
        "      \"canvasMethods\": {},",
        opt_str_array(&o.canvas_methods)
    ));
    lines.push(std::format!(
        "      \"framingBoundaries\": {}",
        opt_num_array(&o.framing_boundaries)
    ));
    lines.push("    }".to_string());
    lines.join("\n")
}

fn source_lines(lines: &mut Vec<String>, entry: &CorpusEntry) {
    match &entry.generator {
        Some(g) => lines.push(std::format!(
            "      \"generator\": {{ \"kind\": {}, \"layer\": {}, \"param\": {} }},",
            json_str(g.kind()),
            g.layer(),
            g.param()
        )),
        None => lines.push(std::format!(
            "      \"bytesHex\": {},",
            json_str(&hex_bytes(&entry.bytes))
        )),
    }
}

fn gate_line(lines: &mut Vec<String>, o: &Outcome) {
    let commands = o
        .layer_commands
        .map(|c| num_array(&c.iter().map(|&v| v as i64).collect::<Vec<_>>()))
        .unwrap_or_else(|| "null".to_string());
    lines.push(std::format!(
        "      \"gate\": {{ \"verdict\": {}, \"error\": {}, \"unknownOpcodes\": {}, \"layersPresent\": {}, \"layerCommands\": {} }},",
        json_str(o.gate_verdict),
        opt_owned(&o.gate_error),
        opt_num(o.unknown.map(|v| v as i64)),
        opt_num(o.present.map(|v| v as i64)),
        commands
    ));
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&std::format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn opt_str(s: Option<&str>) -> String {
    s.map(json_str).unwrap_or_else(|| "null".to_string())
}

fn opt_owned(s: &Option<String>) -> String {
    s.as_deref()
        .map(json_str)
        .unwrap_or_else(|| "null".to_string())
}

fn opt_num(v: Option<i64>) -> String {
    v.map(|n| n.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn num_array(items: &[i64]) -> String {
    let parts: Vec<String> = items.iter().map(|n| n.to_string()).collect();
    std::format!("[{}]", parts.join(", "))
}

fn opt_num_array(items: &Option<Vec<usize>>) -> String {
    match items {
        Some(v) => num_array(&v.iter().map(|&n| n as i64).collect::<Vec<_>>()),
        None => "null".to_string(),
    }
}

fn opt_str_array(items: &Option<Vec<String>>) -> String {
    match items {
        Some(v) => {
            let parts: Vec<String> = v.iter().map(|s| json_str(s)).collect();
            std::format!("[{}]", parts.join(", "))
        }
        None => "null".to_string(),
    }
}
