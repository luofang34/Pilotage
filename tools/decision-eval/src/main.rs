//! `decision-eval PROVIDER_SPEC LABELS.json`: asks a decision provider the
//! survey's picture judgment for every labelled picture and scores it.
//!
//! `LABELS.json` lists pictures relative to its own directory:
//! `[{"image": "frame-ok.jpg", "usable": true}, ...]`. The provider is a
//! [`pilotage_decision::Provider`] specification, such as
//! `http://127.0.0.1:8761/v1/systemone`. The report, one JSON object on
//! standard output, gives accuracy at the 0.5 threshold, the Brier score
//! of the usable probability, the failures, and latency percentiles. The
//! same pictures score every provider, so the numbers compare.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use pilotage_decision::survey::{FrameContext, frame_request, read_frame};
use pilotage_decision::{DecisionClient, Provider};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
enum EvalError {
    #[error("usage: decision-eval PROVIDER_SPEC LABELS.json")]
    Usage,
    #[error("bad provider: {0}")]
    Provider(#[source] pilotage_decision::DecisionError),
    #[error("cannot read {path:?}: {reason}")]
    Read { path: PathBuf, reason: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Label {
    image: String,
    usable: bool,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run_blocking(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            writeln!(std::io::stderr().lock(), "error: {error}").ok();
            ExitCode::FAILURE
        }
    }
}

fn run_blocking(args: &[String]) -> Result<(), EvalError> {
    let [spec, labels] = args else {
        return Err(EvalError::Usage);
    };
    let provider = Provider::from_spec(spec).map_err(EvalError::Provider)?;
    let labels_path = Path::new(labels);
    let read = |path: &Path| {
        std::fs::read(path).map_err(|e| EvalError::Read {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })
    };
    let entries: Vec<Label> =
        serde_json::from_slice(&read(labels_path)?).map_err(|e| EvalError::Read {
            path: labels_path.to_path_buf(),
            reason: e.to_string(),
        })?;
    let directory = labels_path.parent().unwrap_or(Path::new("."));
    let client = DecisionClient::new(provider, Duration::from_secs(60));
    let context = FrameContext {
        task_id: "eval".to_owned(),
        height_m: 110.0,
        nadir: true,
    };
    let (mut correct, mut squared, mut answered, mut failures) =
        (0_usize, Vec::new(), 0_usize, Vec::new());
    let mut latencies = Vec::new();
    for entry in &entries {
        let request = frame_request(read(&directory.join(&entry.image))?, &context);
        let started = Instant::now();
        let outcome = client.decide_blocking(&request);
        latencies.push(started.elapsed().as_secs_f64() * 1000.0);
        match outcome.ok().as_ref().and_then(read_frame) {
            Some(judgment) => {
                answered = answered.wrapping_add(1);
                let truth = if entry.usable { 1.0 } else { 0.0 };
                squared.push((judgment.usable - truth).powi(2));
                if (judgment.usable >= 0.5) == entry.usable {
                    correct = correct.wrapping_add(1);
                } else {
                    failures.push(json!({"image": entry.image, "usable": judgment.usable}));
                }
            }
            None => failures.push(json!({"image": entry.image, "error": "no judgment"})),
        }
    }
    latencies.sort_by(f64::total_cmp);
    let percentile = |p: f64| {
        latencies
            .get(((latencies.len() as f64 - 1.0) * p).round() as usize)
            .copied()
            .unwrap_or(0.0)
    };
    let report = json!({
        "provider": spec,
        "pictures": entries.len(),
        "answered": answered,
        "accuracy": if answered > 0 { correct as f64 / answered as f64 } else { 0.0 },
        "brier": if answered > 0 { squared.iter().sum::<f64>() / answered as f64 } else { 0.0 },
        "latency_ms": {"p50": percentile(0.5), "p90": percentile(0.9), "max": percentile(1.0)},
        "failures": failures,
    });
    writeln!(std::io::stdout().lock(), "{report}").ok();
    Ok(())
}
