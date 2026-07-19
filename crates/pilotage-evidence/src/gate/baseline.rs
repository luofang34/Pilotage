//! Baseline membership of recorded results: the declared source digests
//! must belong to the configuration baseline the result claims.
//!
//! `source.rs` proves a recorded result matches the CURRENT tree; it
//! cannot prove the recorded digests were ever committed as the baseline
//! the result names in `config-digest`. This resolver asks git for
//! `<commit>:<locator>` for every case locator and requires the blob id
//! to equal the declared `source-digest`. A result whose declared
//! baseline never contained those test sources fails closed — and so
//! does a baseline commit absent from the local object store:
//! unverifiable is not verified.

use std::path::Path;
use std::process::Command;

use crate::gate::source::case_locators;
use crate::gate::{Finding, FindingCode};
use crate::graph::Graph;
use crate::id::NodeId;
use crate::node::NodeKind;
use crate::policy::Policy;
use crate::relation::RelationKind;

/// The digest scheme naming a git blob object id.
const GIT_BLOB_SCHEME: &str = "git-blob:";

/// The result attribute naming the configuration commit the run executed
/// against (one of the policy's required result attributes).
const CONFIG_ATTR: &str = "config-digest";

/// Verifies every result's source digest against its declared baseline.
pub(super) fn resolve(
    graph: &Graph,
    policy: &Policy,
    repo_root: &Path,
    findings: &mut Vec<Finding>,
) {
    let toplevel = git_toplevel(repo_root);
    for id in graph.ids_of_kind(NodeKind::VerificationResult) {
        let Some(node) = graph.node(id) else {
            continue;
        };
        // Absent attributes are structural findings elsewhere; this
        // resolver only judges declared pairs.
        let Some(commit) = node.attr(CONFIG_ATTR) else {
            continue;
        };
        let Some(declared) = node.attr(&policy.result_source_attr) else {
            continue;
        };
        let Some(expected) = declared.trim().strip_prefix(GIT_BLOB_SCHEME) else {
            continue; // non-scheme digests are already findings in source.rs
        };
        let commit = commit.trim();
        let Some(toplevel) = &toplevel else {
            push(
                findings,
                id,
                format!(
                    "baseline {commit} cannot be verified: {repo_root:?} is not inside a git \
                     work tree; an unverifiable baseline fails closed"
                ),
            );
            continue;
        };
        // A blob that resolves at `commit` proves nothing if `commit`
        // itself is not reachable from HEAD: a force-push leaves the old
        // commit as a dangling object that `rev-parse` still answers, yet
        // a fresh clone (which fetches only reachable history) can never
        // retrieve it. Require reachability so the recorded baseline is
        // durable, not a force-push artifact.
        match commit_is_reachable(repo_root, commit) {
            Ok(true) => {}
            Ok(false) => {
                push(
                    findings,
                    id,
                    format!(
                        "baseline {commit} is not reachable from HEAD: a non-ancestor commit is \
                         a force-push artifact that a fresh clone cannot fetch; an unreachable \
                         baseline fails closed"
                    ),
                );
                continue;
            }
            Err(reason) => {
                push(
                    findings,
                    id,
                    format!(
                        "baseline {commit} reachability cannot be determined: {reason}; an \
                         unverifiable baseline fails closed"
                    ),
                );
                continue;
            }
        }
        for locator in case_locators(graph, id) {
            let path = locator.split('#').next().unwrap_or(locator);
            check_locator(
                graph, repo_root, toplevel, commit, path, expected, findings, id,
            );
        }
        check_configuration_identity(graph, id, commit, findings);
    }
}

/// The result's declared `config-digest` must equal the commit of a
/// configuration-item that justifies it: a result vouched for by a
/// configuration node recording a DIFFERENT commit carries two
/// contradictory identities, and the trace silently stops meaning
/// anything.
fn check_configuration_identity(
    graph: &Graph,
    id: &NodeId,
    commit: &str,
    findings: &mut Vec<Finding>,
) {
    let mut configuration_commits = graph
        .edges()
        .filter(|edge| edge.from == *id && edge.relation == RelationKind::JustifiedBy)
        .filter_map(|edge| graph.node(&edge.to))
        .filter(|node| node.kind == NodeKind::ConfigurationItem)
        .filter_map(|node| node.attr("commit").map(str::trim))
        .peekable();
    if configuration_commits.peek().is_none() {
        return; // structural completeness is judged elsewhere
    }
    if !configuration_commits.any(|declared| declared == commit) {
        push(
            findings,
            id,
            format!(
                "config-digest {commit} does not match any justifying configuration-item's                  recorded commit; the result claims one baseline while its configuration                  record names another"
            ),
        );
    }
}

/// Asks git whether `commit` contains `path` with the declared blob id.
#[allow(clippy::too_many_arguments)]
fn check_locator(
    _graph: &Graph,
    repo_root: &Path,
    toplevel: &Path,
    commit: &str,
    path: &str,
    expected: &str,
    findings: &mut Vec<Finding>,
    id: &NodeId,
) {
    let full = match crate::gate::contained::resolve_contained(repo_root, path) {
        Ok(full) => full,
        // Escapes are already findings in source.rs; do not double-report.
        Err(_) => return,
    };
    let Ok(relative) = full.strip_prefix(toplevel) else {
        push(
            findings,
            id,
            format!(
                "baseline {commit} cannot vouch for {path}: the locator resolves outside the \
                 git work tree; an unverifiable baseline fails closed"
            ),
        );
        return;
    };
    match git_blob_at(repo_root, commit, &relative.to_string_lossy()) {
        Ok(actual) if actual == expected => {}
        Ok(actual) => push(
            findings,
            id,
            format!(
                "source-digest {GIT_BLOB_SCHEME}{expected} for {path} is not the blob recorded \
                 in baseline {commit} ({GIT_BLOB_SCHEME}{actual}); the declared baseline never \
                 contained this test source"
            ),
        ),
        Err(reason) => push(
            findings,
            id,
            format!(
                "baseline {commit} cannot vouch for {path}: {reason}; an unverifiable baseline \
                 fails closed"
            ),
        ),
    }
}

/// The git work-tree top level containing `repo_root`, if any.
fn git_toplevel(repo_root: &Path) -> Option<std::path::PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| std::path::PathBuf::from(trimmed))
}

/// Whether `commit` is reachable from HEAD (an ancestor of, or equal to,
/// HEAD), or why git could not answer. Reachability — not mere presence
/// in the object store — is what a fresh clone can rely on: `git
/// merge-base --is-ancestor` exits 0 for an ancestor, 1 for a
/// non-ancestor present in the store, and >1 on error (e.g. an unknown
/// commit).
fn commit_is_reachable(repo_root: &Path, commit: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["merge-base", "--is-ancestor", commit, "HEAD"])
        .output()
        .map_err(|error| format!("git could not run ({error})"))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!(
            "git merge-base --is-ancestor {commit} HEAD failed ({})",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

/// The blob object id `commit` records for `relative`, or why git could
/// not answer.
fn git_blob_at(repo_root: &Path, commit: &str, relative: &str) -> Result<String, String> {
    let spec = format!("{commit}:{relative}");
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["rev-parse", "--verify", &spec])
        .output()
        .map_err(|error| format!("git could not run ({error})"))?;
    if !output.status.success() {
        return Err(format!(
            "git does not resolve {spec} (commit missing from the object store, or the path \
             absent at that commit)"
        ));
    }
    let text =
        String::from_utf8(output.stdout).map_err(|_| "git returned non-UTF-8".to_string())?;
    Ok(text.trim().to_string())
}

/// Records a placeholder-result finding against `id`.
fn push(findings: &mut Vec<Finding>, id: &NodeId, detail: String) {
    findings.push(Finding::new(
        FindingCode::PlaceholderResult,
        Some(id.clone()),
        format!("result {id}: {detail}"),
    ));
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::resolve;
    use crate::parse::parse_graph;
    use crate::policy::Policy;

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=evidence-test",
                "-c",
                "user.email=evidence-test@invalid",
            ])
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("utf8")
            .trim()
            .to_string()
    }

    fn graph_text(commit: &str, blob: &str) -> String {
        format!(
            "evidence-graph 1\n\
             scope T\n\
             scope-requirement R1\n\
             node IF intended-function\n\
             node HZ hazard\n\
             node R1 safety-requirement\n\
             node CASE verification-case\n\
             locator sample_tests.rs\n\
             attr test sample\n\
             node RESULT verification-result\n\
             attr command cargo test\n\
             attr config-digest {commit}\n\
             attr source-digest git-blob:{blob}\n\
             edge HZ derives-from IF\n\
             edge R1 mitigates HZ\n\
             edge R1 verified-by CASE\n\
             edge CASE covers R1\n\
             edge RESULT result-of CASE\n\
             edge RESULT covers R1\n"
        )
    }

    fn graph_text_with_configuration(commit: &str, blob: &str, cfg_commit: &str) -> String {
        format!(
            "{}node CFG configuration-item\n\
             attr scm git\n\
             attr commit {cfg_commit}\n\
             edge RESULT justified-by CFG\n",
            graph_text(commit, blob)
        )
    }

    #[test]
    fn a_configuration_item_recording_a_different_commit_is_a_finding() {
        let dir = std::env::temp_dir().join(format!("plt_evg_cfgid_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        git(&dir, &["init", "-q"]);
        std::fs::write(dir.join("sample_tests.rs"), "#[test]\nfn sample() {}\n")
            .expect("write source");
        git(&dir, &["add", "sample_tests.rs"]);
        git(&dir, &["commit", "-qm", "baseline"]);
        let commit = git(&dir, &["rev-parse", "HEAD"]);
        let blob = git(&dir, &["rev-parse", "HEAD:sample_tests.rs"]);
        let policy = Policy::default();

        // Matching identities: no finding.
        let graph = parse_graph(&graph_text_with_configuration(&commit, &blob, &commit))
            .expect("graph parses");
        let mut findings = Vec::new();
        resolve(&graph, &policy, &dir, &mut findings);
        assert!(findings.is_empty(), "matching identity: {findings:?}");

        // The configuration node recording a DIFFERENT commit is the
        // two-baselines lie and must fail closed.
        let mismatched = graph_text_with_configuration(&commit, &blob, &"1".repeat(40));
        let graph = parse_graph(&mismatched).expect("graph parses");
        let mut findings = Vec::new();
        resolve(&graph, &policy, &dir, &mut findings);
        assert!(
            findings
                .iter()
                .any(|finding| finding.detail.contains("does not match any justifying")),
            "mismatched configuration identity must be a finding: {findings:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn baseline_membership_is_verified_against_the_declared_commit() {
        let dir = std::env::temp_dir().join(format!("plt_evg_base_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        git(&dir, &["init", "-q"]);
        std::fs::write(dir.join("sample_tests.rs"), "#[test]\nfn sample() {}\n")
            .expect("write source");
        git(&dir, &["add", "sample_tests.rs"]);
        git(&dir, &["commit", "-qm", "baseline"]);
        let commit = git(&dir, &["rev-parse", "HEAD"]);
        let blob = git(&dir, &["rev-parse", "HEAD:sample_tests.rs"]);
        let policy = Policy::default();

        // The truthful pair verifies.
        let graph = parse_graph(&graph_text(&commit, &blob)).expect("graph parses");
        let mut findings = Vec::new();
        resolve(&graph, &policy, &dir, &mut findings);
        assert!(findings.is_empty(), "truthful baseline: {findings:?}");

        // A digest the declared baseline never contained fails closed.
        let wrong = graph_text(&commit, &"0".repeat(40));
        let graph = parse_graph(&wrong).expect("graph parses");
        let mut findings = Vec::new();
        resolve(&graph, &policy, &dir, &mut findings);
        assert!(
            findings
                .iter()
                .any(|f| f.detail.contains("never contained this test source")),
            "wrong blob: {findings:?}"
        );

        // A baseline commit absent from the object store is unverifiable
        // and fails closed.
        let missing = graph_text(&"1".repeat(40), &blob);
        let graph = parse_graph(&missing).expect("graph parses");
        let mut findings = Vec::new();
        resolve(&graph, &policy, &dir, &mut findings);
        assert!(
            findings
                .iter()
                .any(|f| f.detail.contains("unverifiable baseline fails closed")),
            "missing commit: {findings:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_present_but_unreachable_baseline_fails_closed() {
        // Simulate a force-push orphan: commit, note its id, then amend so
        // the original commit is still in the object store (rev-parse and
        // blob lookups answer) but is no longer an ancestor of HEAD — a
        // fresh clone could never fetch it.
        let dir = std::env::temp_dir().join(format!("plt_evg_reach_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        git(&dir, &["init", "-q"]);
        std::fs::write(dir.join("sample_tests.rs"), "#[test]\nfn sample() {}\n")
            .expect("write source");
        git(&dir, &["add", "sample_tests.rs"]);
        git(&dir, &["commit", "-qm", "baseline"]);
        let orphan = git(&dir, &["rev-parse", "HEAD"]);
        let blob = git(&dir, &["rev-parse", "HEAD:sample_tests.rs"]);
        // Amend to rewrite HEAD; `orphan` is now a dangling object.
        git(
            &dir,
            &["commit", "-q", "--amend", "-m", "baseline (amended)"],
        );
        let head = git(&dir, &["rev-parse", "HEAD"]);
        assert_ne!(orphan, head, "amend rewrote the commit");
        // The orphan's blob still resolves — presence is not reachability.
        assert_eq!(
            super::git_blob_at(&dir, &orphan, "sample_tests.rs").expect("blob resolves"),
            blob,
            "the orphan is still present in the object store"
        );

        let graph = parse_graph(&graph_text(&orphan, &blob)).expect("graph parses");
        let mut findings = Vec::new();
        resolve(&graph, &Policy::default(), &dir, &mut findings);
        assert!(
            findings
                .iter()
                .any(|f| f.detail.contains("not reachable from HEAD")),
            "a present-but-unreachable baseline must fail closed: {findings:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
