//! Every pull-request-triggered workflow cancels superseded runs.
//!
//! A push to a pull request supersedes the runs of the push before it;
//! a workflow without a concurrency group lets the stale run keep the
//! runners the repository has few of. A workflow that never runs for
//! pull requests (a schedule, a manual dispatch) is exempt: its runs
//! are not superseded by pushes.

#![allow(clippy::expect_used, clippy::panic)]

/// True when the workflow runs for pull requests: a block-style
/// `pull_request` or `pull_request_target` trigger, or a flow-style
/// `on:` line that names one.
fn pull_request_triggered(text: &str) -> bool {
    text.lines().any(|line| {
        line.starts_with("  pull_request")
            || (line.starts_with("on:") && line.contains("pull_request"))
    })
}

/// True when the workflow declares a top-level concurrency group that
/// cancels the superseded run. The cancel line is pinned at top level:
/// a job-level cancel would leave the other jobs' runners held.
fn cancels_superseded(text: &str) -> bool {
    let has_group = text.lines().any(|line| line.trim_end() == "concurrency:");
    let cancels = text
        .lines()
        .any(|line| line.trim_end() == "  cancel-in-progress: true");
    has_group && cancels
}

/// True when the group is keyed by the pull-request number with a
/// fallback. Any other key could put two pushes to main in one group
/// and cancel a main run.
fn groups_by_pull_request(text: &str) -> bool {
    text.lines().any(|line| {
        line.trim_start().starts_with("group:")
            && line.contains("github.event.pull_request.number ||")
    })
}

#[test]
fn a_pull_request_workflow_without_a_group_is_a_violation() {
    for workflow in [
        "on:\n  push:\n    branches: [main]\n  pull_request:\n\njobs:\n",
        "on:\n  pull_request_target:\n\njobs:\n",
        "on: [push, pull_request]\n\njobs:\n",
    ] {
        assert!(pull_request_triggered(workflow), "{workflow:?}");
        assert!(!cancels_superseded(workflow), "{workflow:?}");
    }
}

#[test]
fn a_job_level_cancel_does_not_satisfy_the_obligation() {
    let workflow = "on:\n  pull_request:\n\nconcurrency:\n  group: x\n\njobs:\n  a:\n    concurrency:\n      cancel-in-progress: true\n";
    assert!(!cancels_superseded(workflow));
}

#[test]
fn a_grouped_workflow_and_an_exempt_workflow_pass() {
    let grouped = "on:\n  pull_request:\n\nconcurrency:\n  group: x-${{ github.event.pull_request.number || github.sha }}\n  cancel-in-progress: true\n\njobs:\n";
    assert!(pull_request_triggered(grouped));
    assert!(cancels_superseded(grouped));
    assert!(groups_by_pull_request(grouped));

    let ref_keyed = "concurrency:\n  group: x-${{ github.ref }}\n  cancel-in-progress: true\n";
    assert!(!groups_by_pull_request(ref_keyed));

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
        if path
            .extension()
            .is_none_or(|ext| ext != "yml" && ext != "yaml")
        {
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
        assert!(
            groups_by_pull_request(&text),
            "{} must key its concurrency group by \
             github.event.pull_request.number with a fallback, so a push \
             to main never shares a group with another",
            path.display()
        );
    }
    assert!(
        checked >= 2,
        "expected at least the CI and evidence-gate workflows, found {checked}"
    );
}
