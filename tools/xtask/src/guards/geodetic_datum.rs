//! One datum vocabulary reaches every reader of a geodetic fix.
//!
//! The wire carries a datum as a number. The Rust producer, the proto
//! comment a reader implements from, and the browser decoders each hold
//! their own copy of what those numbers mean. A variant added to one copy
//! and not another is a position read against the wrong reference, which
//! is the failure ADR-0022 exists to prevent, so the copies are compared
//! here, with the gates each lane needs on its way to the wire.

use std::path::Path;

use crate::error::XtaskError;

#[cfg(test)]
mod tests;

/// The guard's name in the guard report.
pub(crate) const NAME: &str = "geodetic-datum-codes";

pub(crate) const DATUM: &str = "crates/pilotage-geo/src/datum.rs";
pub(crate) const PROTO: &str = "schemas/pilotage/v1/telemetry.proto";
pub(crate) const DECODER: &str = "clients/web/wire.js";
/// The browser reads telemetry through the wasm decoder, not through
/// wire.js: a field only the JavaScript decoder knows is a field the
/// client never sees, and no decode error says so.
pub(crate) const WASM_DECODER: &str = "clients/web-instruments/src/decode_envelope/groups.rs";
pub(crate) const MAPPER: &str = "crates/pilotage-host-core/src/engine_actor/telemetry.rs";
pub(crate) const ATTACHMENTS: &str = "adapters/aviate/src/adapter/sim_attachments.rs";
pub(crate) const CAMERA: &str = "adapters/aviate/src/adapter/camera.rs";
pub(crate) const LAUNCHER: &str = "tools/xtask/src/backend/aviate_gz.rs";
pub(crate) const CI: &str = ".github/workflows/ci.yml";

/// The sources the guard compares.
#[derive(Debug, Clone)]
pub(crate) struct Sources {
    pub(crate) datum: String,
    pub(crate) proto: String,
    pub(crate) decoder: String,
    pub(crate) wasm_decoder: String,
    pub(crate) mapper: String,
    pub(crate) attachments: Option<String>,
    pub(crate) camera: Option<String>,
    pub(crate) launcher: Option<String>,
    pub(crate) ci: String,
}

impl Sources {
    /// Reads the sources under `root`.
    ///
    /// # Errors
    ///
    /// [`XtaskError::Io`] when a required file cannot be read.
    pub(crate) fn read(root: &Path) -> Result<Self, XtaskError> {
        let required = |path: &str| {
            std::fs::read_to_string(root.join(path)).map_err(|source| XtaskError::Io {
                context: "reading a required geodetic file for the datum guard",
                source,
            })
        };
        let optional = |path: &str| std::fs::read_to_string(root.join(path)).ok();
        Ok(Self {
            datum: required(DATUM)?,
            proto: required(PROTO)?,
            decoder: required(DECODER)?,
            wasm_decoder: required(WASM_DECODER)?,
            mapper: required(MAPPER)?,
            attachments: optional(ATTACHMENTS),
            camera: optional(CAMERA),
            launcher: optional(LAUNCHER),
            ci: required(CI)?,
        })
    }
}

/// Every refusal for the sources under `root`.
///
/// # Errors
///
/// [`XtaskError::Io`] when a required file cannot be read.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, XtaskError> {
    Ok(refusals(&Sources::read(root)?))
}

/// Every refusal for `sources`.
pub(crate) fn refusals(sources: &Sources) -> Vec<String> {
    let mut out = Vec::new();
    check_enum(sources, "HorizontalDatum", "horizontalDatum", &mut out);
    check_enum(sources, "VerticalDatum", "verticalDatum", &mut out);
    check_mapper(&sources.mapper, &mut out);
    check_decoders(sources, &mut out);
    check_sim_lane(sources, &mut out);
    check_ci(&sources.ci, &mut out);
    out
}

/// The proto comment is the contract a reader implements from, so it must
/// name every code the crate can produce, and the browser must refuse
/// anything past the last one.
fn check_enum(sources: &Sources, name: &str, decoder_name: &str, out: &mut Vec<String>) {
    let top = highest_code(&sources.datum, name);
    if top < 1 {
        out.push(format!("FORBIDDEN: {name} declares no wire codes"));
        return;
    }
    let comment = proto_comment(&sources.proto, name);
    for code in 1..=top {
        if !names_code(&comment, code) {
            out.push(format!(
                "FORBIDDEN: telemetry.proto does not name {name} code {code}"
            ));
        }
    }
    if !sources.decoder.contains(&format!("{decoder_name} > {top}")) {
        out.push(format!(
            "FORBIDDEN: the browser must refuse a {name} code past {top}"
        ));
    }
}

/// The highest code the enum's `from_u8` decoder accepts: the crate's
/// own statement of how many variants can reach the wire.
pub(crate) fn highest_code(datum: &str, name: &str) -> u32 {
    let header = format!("pub enum {name} {{");
    let mut inside = false;
    let mut top = 0;
    for line in datum.lines() {
        if line.contains(&header) {
            inside = true;
        }
        if !inside {
            continue;
        }
        if line.contains("=> Some(Self::") {
            let code = line
                .split_whitespace()
                .next()
                .and_then(|token| token.parse::<u32>().ok())
                .unwrap_or(0);
            top = top.max(code);
        }
        if line.contains("_ => None") {
            break;
        }
    }
    top
}

/// The eight lines of the proto from the first that names the enum.
fn proto_comment(proto: &str, name: &str) -> String {
    let marker = format!("pilotage_geo::{name}");
    proto
        .lines()
        .skip_while(|line| !line.contains(&marker))
        .take(8)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether `comment` names `code`: the number, not part of a larger one,
/// then a space and a letter.
fn names_code(comment: &str, code: u32) -> bool {
    let needle = format!("{code} ");
    comment.match_indices(&needle).any(|(at, _)| {
        let before = comment.get(..at).and_then(|text| text.chars().next_back());
        let after = comment
            .get(at + needle.len()..)
            .and_then(|text| text.chars().next());
        before.is_none_or(|c| !c.is_ascii_digit()) && after.is_some_and(|c| c.is_ascii_alphabetic())
    })
}

fn check_mapper(mapper: &str, out: &mut Vec<String>) {
    // The producer converts through the typed codec, so a discriminant
    // that moves cannot be re-numbered by an `as` cast.
    if casts_datum(mapper) {
        out.push("FORBIDDEN: a datum must reach the wire through to_u8(), not a cast".to_owned());
    }
    require(
        mapper.contains("position.horizontal_datum.to_u8()")
            && mapper.contains("vertical.datum.to_u8()"),
        "the wire mapping must convert datums through to_u8()",
        out,
    );
    // An absent fix stays absent: a default is Null Island, a real place.
    require(
        !mapper.contains("GeodeticFix::default"),
        "an absent geodetic fix must not take a default",
        out,
    );
    // Both lanes reach the wire through the one mapping that re-runs the
    // contract: two call sites, so the count is what is checked.
    require(
        mapper
            .lines()
            .filter(|line| line.contains("and_then(geodetic_to_wire)"))
            .count()
            == 2,
        "each lane must reach the wire through the mapping, and only through it",
        out,
    );
    // The constructor wraps a longitude into range; the mapping sends
    // that value, or the readers disagree about the same bytes.
    require(
        mapper.contains("position.normalized()"),
        "the wire mapping must send the normalized position",
        out,
    );
    require(
        mapper.contains("SIMULATOR_GEOID_MODEL_ID")
            && mapper.contains("SourceRole::SimulationTruth"),
        "the simulator separation must be refused outside the truth role",
        out,
    );
    require(
        mapper.contains("fix.stamp.role == SourceRole::OperationalEstimate"),
        "the estimate lane must refuse another role at the producer",
        out,
    );
    require(
        mapper.contains("fix.stamp.role == SourceRole::SimulationTruth"),
        "the truth lane must refuse another role at the producer",
        out,
    );
}

/// Whether the mapping casts a datum onto the wire with `as u32`:
/// `horizontal_datum as u32`, `.datum as u32`, or `vertical` and one
/// character before `datum as u32`.
fn casts_datum(mapper: &str) -> bool {
    let needle = "datum as u32";
    mapper.match_indices(needle).any(|(at, _)| {
        let before = mapper.get(..at).unwrap_or_default();
        let without_last = before
            .char_indices()
            .next_back()
            .and_then(|(index, _)| before.get(..index))
            .unwrap_or_default();
        before.ends_with("horizontal_")
            || before.ends_with('.')
            || without_last.ends_with("vertical")
    })
}

fn check_decoders(sources: &Sources, out: &mut Vec<String>) {
    require(
        sources.decoder.contains("geodeticStamp.role !== 1"),
        "the estimate lane must gate its fix on the estimate role",
        out,
    );
    require(
        sources
            .wasm_decoder
            .contains("SourceRole::OperationalEstimate"),
        "the decoder the client runs must gate the estimate lane",
        out,
    );
    for lane in ["geodetic", "geodetic_stamp"] {
        require(
            sources.wasm_decoder.contains(&format!("{lane},")),
            &format!("the decoder the client runs must surface {lane}"),
            out,
        );
    }
    require(
        sources.wasm_decoder.contains("fix.longitude_deg < 180.0"),
        "the decoder the client runs must refuse an unnormalized longitude",
        out,
    );
    // An origin is a u64; read as a Number it collapses past 2^53.
    require(
        sources.decoder.contains("firstBigVarint(f, 10)"),
        "the local origin must decode through the 64-bit path",
        out,
    );
}

/// The Gazebo lane joins a sensor fix to the truth sample it belongs
/// with, on the topic of the world the launcher started.
fn check_sim_lane(sources: &Sources, out: &mut Vec<String>) {
    if let Some(attachments) = &sources.attachments {
        require(
            attachments.contains("sample.geodetic = self.paired_fix(sample.stamp)"),
            "the truth sample must carry the fix the sensor paired with it",
            out,
        );
        require(
            attachments.contains("latest_cached().navsat"),
            "the paired fix must come from the sensor cache",
            out,
        );
    }
    if let (Some(camera), Some(launcher)) = (&sources.camera, &sources.launcher) {
        require(
            camera.contains("sensor/navsat_sensor/navsat"),
            "the sensor topic must name the model's navsat sensor",
            out,
        );
        require(
            camera.contains("PILOTAGE_GZ_WORLD") && launcher.contains("PILOTAGE_GZ_WORLD"),
            "the launcher and the reader must agree on the world name",
            out,
        );
    }
}

/// The guardrails hold only while CI runs them: the browser's geodetic
/// test, and the guard runner that runs this guard.
fn check_ci(ci: &str, out: &mut Vec<String>) {
    let runs: Vec<&str> = ci
        .lines()
        .filter(|line| line.trim_start().starts_with("run:"))
        .collect();
    require(
        runs.iter()
            .any(|line| line.contains("clients/web/geodetic-fix.test.mjs")),
        "CI must run clients/web/geodetic-fix.test.mjs",
        out,
    );
    require(
        runs.iter().any(|line| line.contains("-- guards")),
        "CI must run the guards",
        out,
    );
}

fn require(holds: bool, what: &str, out: &mut Vec<String>) {
    if !holds {
        out.push(format!("FORBIDDEN: {what}"));
    }
}
