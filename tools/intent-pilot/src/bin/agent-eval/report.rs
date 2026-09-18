//! The result files and the summary table.

use std::path::Path;

use pilotage_agent::{AdapterDeclaration, CaseResult, Suite, SuiteReport, Tally};
use serde::Serialize;

use crate::error::EvalError;

#[derive(Serialize)]
struct Summary<'a> {
    suite_id: &'a str,
    suite_sha256: &'a str,
    held_out: bool,
    run_number: usize,
    adapter: &'a AdapterDeclaration,
    report: &'a SuiteReport,
}

/// Writes `cases.jsonl` and `summary.json` into `out`.
pub(crate) async fn write(
    out: &Path,
    suite: &Suite,
    suite_sha256: &str,
    adapter: &AdapterDeclaration,
    run_number: usize,
    results: &[CaseResult],
    report: &SuiteReport,
) -> Result<(), EvalError> {
    let failed = |path: &Path| {
        let path = path.to_owned();
        move |source| EvalError::Write { path, source }
    };
    tokio::fs::create_dir_all(out).await.map_err(failed(out))?;
    let mut cases = Vec::new();
    for result in results {
        let line = serde_json::to_vec(result).map_err(|e| failed(out)(e.into()))?;
        cases.extend_from_slice(&line);
        cases.push(b'\n');
    }
    let cases_path = out.join("cases.jsonl");
    tokio::fs::write(&cases_path, cases)
        .await
        .map_err(failed(&cases_path))?;
    let summary = Summary {
        suite_id: &suite.id,
        suite_sha256,
        held_out: suite.held_out,
        run_number,
        adapter,
        report,
    };
    let text = serde_json::to_vec_pretty(&summary).map_err(|e| failed(out)(e.into()))?;
    let summary_path = out.join("summary.json");
    tokio::fs::write(&summary_path, text)
        .await
        .map_err(failed(&summary_path))
}

/// The summary as a text table.
pub(crate) fn table(
    suite: &Suite,
    suite_sha256: &str,
    adapter: &AdapterDeclaration,
    run_number: usize,
    report: &SuiteReport,
) -> String {
    let digest = suite_sha256.get(..12).unwrap_or(suite_sha256);
    let held_out = if suite.held_out { "held-out" } else { "open" };
    let mut text = format!(
        "suite   {} ({held_out}, sha256 {digest})\nadapter {}\nmodel   {}\nrun     {run_number} of this adapter and model on this suite digest\n\n",
        suite.id, adapter.adapter, adapter.model
    );
    text.push_str(&row("ALL CASES", report.cases));
    text.push_str("by directive kind\n");
    for (kind, tally) in &report.by_kind {
        text.push_str(&row(&format!("  {kind:?}"), *tally));
    }
    text.push_str("by category\n");
    for (category, tally) in &report.by_category {
        text.push_str(&row(&format!("  {category}"), *tally));
    }
    text.push_str("by slot\n");
    for (slot, tally) in &report.by_slot {
        text.push_str(&row(&format!("  {slot}"), *tally));
    }
    text.push_str(&format!(
        "\nunable {}   refused {}   faults {}   median model time {:.0} ms\n",
        report.unable, report.refused, report.faults, report.median_model_ms
    ));
    text.push_str(&format!(
        "wrong answers: {} would be flown, {} stopped by the grounding check; correct answers stopped: {}\n",
        report.wrong_and_flown, report.wrong_and_stopped, report.correct_and_stopped
    ));
    let probability =
        |value: Option<f64>| value.map_or_else(|| "n/a".to_owned(), |p| format!("{p:.2}"));
    text.push_str(&format!(
        "slot probability: smallest when correct {}, largest when wrong {}\n",
        probability(report.min_probability_when_correct),
        probability(report.max_probability_when_wrong)
    ));
    text
}

fn row(label: &str, tally: Tally) -> String {
    format!("{label:<28} {:>3} / {:<3}\n", tally.correct, tally.total)
}
