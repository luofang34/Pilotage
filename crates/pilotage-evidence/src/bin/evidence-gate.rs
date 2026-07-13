//! `evidence-gate`: run the scoped no-orphan gate over an evidence graph.
//!
//! ```text
//! evidence-gate [--graph PATH] [--repo-root PATH] [--resolve-selectors]
//!               [--impact NODE_ID]
//! ```
//!
//! The gate is an engineering trace check. A clean run is **not** a DO-178C,
//! ISO 26262, ECSS, TSO, or ASIL result and is not tool qualification. The
//! process exits non-zero when the verdict is INVALID so CI can observe it, but
//! callers must not wire it as a required certification gate.

use std::env;
use std::fs;
use std::io::{self, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;

use pilotage_evidence::gate::{validate, validate_resolving};
use pilotage_evidence::policy::Policy;
use pilotage_evidence::{EvidenceError, NodeId, impact, parse, report};

const DEFAULT_GRAPH: &str = "docs/instruments/evidence-graph.evg";

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(err) => {
            writeln!(io::stderr(), "evidence-gate: {err}").ok();
            ExitCode::FAILURE
        }
    }
}

/// Parsed command-line options.
struct Options {
    graph: Option<PathBuf>,
    repo_root: Option<PathBuf>,
    resolve_selectors: bool,
    impact: Option<String>,
}

fn run(args: Vec<String>) -> Result<ExitCode, EvidenceError> {
    let options = match parse_args(&args) {
        Ok(options) => options,
        Err(message) => {
            writeln!(io::stderr(), "evidence-gate: {message}\n{USAGE}").ok();
            return Ok(ExitCode::FAILURE);
        }
    };
    let repo_root = options
        .repo_root
        .or_else(|| env::var_os("PILOTAGE_REPO_ROOT").map(PathBuf::from))
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let graph_path = options
        .graph
        .unwrap_or_else(|| repo_root.join(DEFAULT_GRAPH));

    let text = fs::read_to_string(&graph_path).map_err(|source| EvidenceError::Read {
        path: graph_path.display().to_string(),
        source,
    })?;
    let graph = parse::parse_graph(&text)?;
    let policy = Policy::engineering_trace();

    let gate_report = if options.resolve_selectors {
        validate_resolving(&graph, &policy, &repo_root)
    } else {
        validate(&graph, &policy)
    };

    let mut stdout = io::stdout().lock();
    write!(stdout, "{}", report::render_gate(&gate_report, &graph)).ok();

    if let Some(raw) = options.impact {
        match NodeId::new(raw) {
            Ok(id) => {
                let impact_report = impact::impact(&graph, &id);
                write!(stdout, "\n{}", report::render_impact(&impact_report)).ok();
            }
            Err(err) => {
                writeln!(io::stderr(), "evidence-gate: bad --impact id: {err}").ok();
                return Ok(ExitCode::FAILURE);
            }
        }
    }

    Ok(if gate_report.verdict.passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

const USAGE: &str = "usage: evidence-gate [--graph PATH] [--repo-root PATH] \
[--resolve-selectors] [--impact NODE_ID]";

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        graph: None,
        repo_root: None,
        resolve_selectors: false,
        impact: None,
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--graph" => options.graph = Some(PathBuf::from(next(&mut iter, "--graph")?)),
            "--repo-root" => {
                options.repo_root = Some(PathBuf::from(next(&mut iter, "--repo-root")?))
            }
            "--resolve-selectors" => options.resolve_selectors = true,
            "--impact" => options.impact = Some(next(&mut iter, "--impact")?),
            "-h" | "--help" => return Err("help requested".to_string()),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(options)
}

fn next(iter: &mut std::slice::Iter<'_, String>, flag: &str) -> Result<String, String> {
    iter.next()
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}
