//! Every pull-request-triggered workflow cancels superseded runs.
//!
//! A push to a pull request supersedes the runs of the push before it;
//! a workflow without a concurrency group lets the stale run keep the
//! runners the repository has few of. A workflow that never runs for
//! pull requests (a schedule, a manual dispatch) is exempt: its runs
//! are not superseded by pushes.

#![allow(clippy::expect_used, clippy::panic)]

/// True when the workflow runs for pull requests.
fn pull_request_triggered(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim_end() == "  pull_request:" || line.starts_with("  pull_request:"))
}

/// True when the workflow declares a top-level concurrency group that
/// cancels the superseded run.
fn cancels_superseded(text: &str) -> bool {
    let has_group = text.lines().any(|line| line.trim_end() == "concurrency:");
    let cancels = text
        .lines()
        .any(|line| line.trim() == "cancel-in-progress: true");
    has_group && cancels
}

#[test]
fn a_pull_request_workflow_without_a_group_is_a_violation() {
    let workflow = "on:\n  push:\n    branches: [main]\n  pull_request:\n\njobs:\n";
    assert!(pull_request_triggered(workflow));
    assert!(!cancels_superseded(workflow));
}

#[test]
fn a_grouped_workflow_and_an_exempt_workflow_pass() {
    let grouped = "on:\n  pull_request:\n\nconcurrency:\n  group: x-${{ github.sha }}\n  cancel-in-progress: true\n\njobs:\n";
    assert!(pull_request_triggered(grouped));
    assert!(cancels_superseded(grouped));

    let scheduled = "on:\n  schedule:\n    - cron: \"0 9 * * *\"\n  workflow_dispatch:\n\njobs:\n";
    assert!(!pull_request_triggered(scheduled));
}

#[test]
fn repository_workflows_cancel_superseded_runs() {
    let workflows = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.github/workflows")
        .canonicalize()
        .expect("workflows directory");
    let mut checked = 0;
    for entry in std::fs::read_dir(&workflows).expect("read workflows") {
        let path = entry.expect("directory entry").path();
        if path.extension().is_none_or(|ext| ext != "yml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read workflow");
        if !pull_request_triggered(&text) {
            continue;
        }
        checked += 1;
        assert!(
            cancels_superseded(&text),
            "{} runs for pull requests but does not cancel superseded runs; \
             declare a concurrency group with cancel-in-progress: true",
            path.display()
        );
    }
    assert!(
        checked >= 2,
        "expected at least the CI and evidence-gate workflows, found {checked}"
    );
}
