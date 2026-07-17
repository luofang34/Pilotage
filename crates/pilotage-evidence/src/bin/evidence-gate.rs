//! `evidence-gate`: run the scoped no-orphan gate over an evidence graph.
//!
//! ```text
//! evidence-gate [--graph PATH] [--repo-root PATH] [--resolve-selectors]
//!               [--require-resolvable] [--impact NODE_ID] [--trace]
//! ```
//!
//! `--require-resolvable` is a hard check meant for a required (not
//! advisory) CI job: every recorded result must resolve — its baseline
//! reachable from HEAD, its selectors and artifacts present — and the
//! process exits non-zero if any does not. A review that is honestly
//! PENDING is tolerated, so the check enforces durable evidence
//! resolution without asserting the review is complete.
//!
//! The gate is an engineering trace check. A clean run is **not** a DO-178C,
//! ISO 26262, ECSS, TSO, or ASIL result and is not tool qualification. The
//! process exits non-zero when the verdict is INVALID so CI can observe it, but
//! callers must not wire it as a required certification gate.
//!
//! Exception expiry is enforced: the as-of date defaults to today (UTC) and can
//! be pinned with `--as-of YYYY-MM-DD` for reproducible checks. An exception
//! whose expiry precedes the as-of date can no longer suppress a finding.

use std::env;
use std::fs;
use std::io::{self, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use pilotage_evidence::gate::{FindingCode, validate, validate_resolving};
use pilotage_evidence::policy::Policy;
use pilotage_evidence::{EvidenceError, NodeId, impact, parse, report, trace};

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
    require_resolvable: bool,
    impact: Option<String>,
    trace: bool,
    as_of: Option<String>,
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
    let as_of = options.as_of.unwrap_or_else(today_utc);
    let policy = Policy {
        exception_as_of: Some(as_of.clone()),
        ..Policy::engineering_trace()
    };

    // Resolvability (baseline reachability, selector, artifact) is filesystem-
    // and git-backed, so it needs the repo root; `--require-resolvable` implies it.
    let gate_report = if options.resolve_selectors || options.require_resolvable {
        validate_resolving(&graph, &policy, &repo_root)
    } else {
        validate(&graph, &policy)
    };

    let mut stdout = io::stdout().lock();
    write!(stdout, "{}", report::render_gate(&gate_report, &graph)).ok();
    writeln!(stdout, "exception expiry enforced as of: {as_of}").ok();

    if options.trace {
        let resolution = trace::resolve(&graph);
        write!(stdout, "\n{}", report::render_trace(&resolution, &graph)).ok();
    }

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

    if options.require_resolvable {
        return Ok(resolvable_exit(&gate_report));
    }

    Ok(if gate_report.verdict.passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// The exit code for `--require-resolvable`: a hard, non-advisory check
/// that every result resolves — its baseline reachable, its selectors and
/// artifacts present. The review verdict may honestly stay PENDING, so a
/// `ReviewIncomplete` finding is tolerated; every other non-excepted
/// finding fails. Safe to wire as a required CI step that a broken or
/// orphaned baseline can never leave green.
fn resolvable_exit(gate_report: &pilotage_evidence::gate::GateReport) -> ExitCode {
    let blocking = gate_report
        .findings
        .iter()
        .filter(|f| !f.excepted && f.code != FindingCode::ReviewIncomplete)
        .count();
    if blocking > 0 {
        writeln!(
            io::stderr(),
            "evidence-gate: {blocking} unresolved finding(s) fail --require-resolvable \
             (review-pending is tolerated; broken baselines/selectors/artifacts are not)"
        )
        .ok();
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

const USAGE: &str = "usage: evidence-gate [--graph PATH] [--repo-root PATH] \
[--resolve-selectors] [--require-resolvable] [--impact NODE_ID] [--trace] [--as-of YYYY-MM-DD]";

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        graph: None,
        repo_root: None,
        resolve_selectors: false,
        require_resolvable: false,
        impact: None,
        trace: false,
        as_of: None,
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--graph" => options.graph = Some(PathBuf::from(next(&mut iter, "--graph")?)),
            "--repo-root" => {
                options.repo_root = Some(PathBuf::from(next(&mut iter, "--repo-root")?))
            }
            "--resolve-selectors" => options.resolve_selectors = true,
            "--require-resolvable" => options.require_resolvable = true,
            "--impact" => options.impact = Some(next(&mut iter, "--impact")?),
            "--trace" => options.trace = true,
            "--as-of" => options.as_of = Some(next(&mut iter, "--as-of")?),
            "-h" | "--help" => return Err("help requested".to_string()),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(options)
}

/// Today's date (UTC) as `YYYY-MM-DD`, or the Unix epoch if the clock is
/// unreadable — an unreadable clock treats every exception as expired, which is
/// fail-closed rather than silently skipping the expiry check.
fn today_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    utc_date(i64::try_from(secs).unwrap_or(0))
}

/// Converts a Unix timestamp (seconds) to a UTC `YYYY-MM-DD` string using the
/// civil-from-days algorithm, so no date crate or wall clock is needed here and
/// the conversion is pure and testable.
fn utc_date(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}

fn next(iter: &mut std::slice::Iter<'_, String>, flag: &str) -> Result<String, String> {
    iter.next()
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::utc_date;

    #[test]
    fn utc_date_converts_known_timestamps() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(1_577_836_800), "2020-01-01");
        // 2026-07-14T00:00:00Z
        assert_eq!(utc_date(1_783_987_200), "2026-07-14");
        // 2026-07-14T23:59:59Z is still the same civil day.
        assert_eq!(utc_date(1_784_073_599), "2026-07-14");
    }
}
